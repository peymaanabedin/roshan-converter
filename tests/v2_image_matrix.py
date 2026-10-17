#!/usr/bin/env python3
"""Exercise the real Rust CLI image pipeline with redistributable synthetic data.

Requires Python, Pillow, NumPy, g++, and the built Linux native codec prefix.
The tested CLI can be Linux native or a Windows executable through --wine.
No network, user photos, or system image codecs are used. All generated files
remain under --work-dir; only the JSON evidence belongs in the source bundle.
"""
from __future__ import annotations
import argparse
import ctypes as C
import hashlib
import io
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import zlib
import numpy as np
from PIL import Image, ImageCms, ImageOps


def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def require(condition, message):
    if not condition: raise AssertionError(message)
def generated_profile_equal(actual, expected):
    # ICC bytes 40..43 identify the creator platform (lcms emits MSFT on Windows,
    # APPL on Linux). All colorimetric/profile data must remain byte-identical.
    return len(actual)==len(expected) and actual[:40]==expected[:40] and actual[44:]==expected[44:]
def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
def chunks(data):
    at = 8
    while at + 12 <= len(data):
        length = struct.unpack_from('>I', data, at)[0]
        yield data[at + 4:at + 8], data[at + 8:at + 8 + length]
        at += 12 + length
def save_png(path, array, icc=None, exif=None, extra=()):
    h, w, channels = array.shape
    bits = 16 if array.dtype.itemsize == 2 else 8
    header = struct.pack('>IIBBBBB', w, h, bits, 6 if channels == 4 else 2, 0, 0, 0)
    rows = array.astype('>u2').tobytes() if bits == 16 else array.tobytes()
    stride = w * channels * (bits // 8)
    data = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header)
    for kind, value in extra: data += chunk(kind, value)
    if icc: data += chunk(b'iCCP', b'Roshan QA\x00\x00' + zlib.compress(icc))
    if exif: data += chunk(b'eXIf', exif.removeprefix(b'Exif\0\0'))
    data += chunk(b'IDAT', zlib.compress(b''.join(b'\0' + rows[y * stride:(y + 1) * stride] for y in range(h))))
    path.write_bytes(data + chunk(b'IEND', b''))


def read_png_samples(path):
    items = list(chunks(path.read_bytes()))
    w, h, bits, kind, _, _, interlace = struct.unpack('>IIBBBBB', next(v for k,v in items if k == b'IHDR'))
    require(kind in (2,6) and interlace == 0, 'Test expects non-interlaced RGB/RGBA PNG')
    count = 4 if kind == 6 else 3
    stride, bpp = w * count * bits // 8, count * bits // 8
    packed = zlib.decompress(b''.join(v for k,v in items if k == b'IDAT'))
    previous = bytearray(stride)
    rows = bytearray()
    def paeth(a,b,c):
        p = a + b - c; distances = [abs(p-a),abs(p-b),abs(p-c)]
        return [a,b,c][distances.index(min(distances))]
    for y in range(h):
        at = y * (stride + 1); mode = packed[at]; row = bytearray(packed[at+1:at+1+stride])
        for i in range(stride):
            left = row[i-bpp] if i >= bpp else 0; up = previous[i]
            corner = previous[i-bpp] if i >= bpp else 0
            predictor = 0 if mode == 0 else left if mode == 1 else up if mode == 2 else (left+up)//2 if mode == 3 else paeth(left,up,corner)
            row[i] = (row[i] + predictor) & 255
        previous = row; rows.extend(row)
    return np.frombuffer(rows, dtype='>u2' if bits == 16 else np.uint8).astype(np.uint16 if bits == 16 else np.uint8).reshape(h,w,count)


class NativeImage(C.Structure):
    _fields_ = [('width',C.c_uint32),('height',C.c_uint32),('pixels',C.c_void_p),('pixels_len',C.c_size_t),
                ('icc',C.c_void_p),('icc_len',C.c_size_t),('exif',C.c_void_p),('exif_len',C.c_size_t),('flags',C.c_uint32)]
class NativeBuffer(C.Structure):
    _fields_ = [('data',C.c_void_p),('len',C.c_size_t),('flags',C.c_uint32)]
class Inspector:
    def __init__(self, root, prefix, work, cxx):
        library = work / 'inspection-bridge.so'
        env = os.environ.copy(); env.setdefault('TMPDIR',str(work))
        subprocess.run([cxx,'-std=c++17','-O2','-pipe','-shared','-fPIC','-DLIBHEIF_STATIC_BUILD',
                        '-I'+str(prefix/'include'), str(root/'native/bridge.cpp'), '-L'+str(prefix/'lib'),
                        '-lheif','-lde265','-lx265','-llcms2','-lpthread','-lm','-ldl','-o',str(library)],check=True,env=env)
        self.lib=C.CDLL(str(library)); l=self.lib
        l.rh_open.argtypes=[C.c_void_p,C.c_size_t,C.c_void_p,C.c_size_t]; l.rh_open.restype=C.c_void_p
        l.rh_decode_rgba.argtypes=[C.c_void_p,C.c_int,C.POINTER(NativeImage),C.c_void_p,C.c_size_t]
        l.rh_close.argtypes=[C.c_void_p]; l.rh_free_image.argtypes=[C.POINTER(NativeImage)]
        l.rh_nclx_profile.argtypes=[C.c_uint32]*4+[C.POINTER(NativeBuffer),C.c_void_p,C.c_size_t]
        l.rh_free_buffer.argtypes=[C.POINTER(NativeBuffer)]
    def profile(self,codes):
        out=NativeBuffer(); error=C.create_string_buffer(1024)
        require(self.lib.rh_nclx_profile(*codes,C.byref(out),error,1024)==1,error.value.decode())
        try:return C.string_at(out.data,out.len)
        finally:self.lib.rh_free_buffer(C.byref(out))
    def read(self,path):
        if path.suffix.lower() in ('.heic','.heif'):
            source=C.create_string_buffer(path.read_bytes()); error=C.create_string_buffer(1024)
            document=self.lib.rh_open(source,len(source)-1,error,1024)
            require(bool(document),error.value.decode()); image=NativeImage()
            try:
                require(self.lib.rh_decode_rgba(document,0,C.byref(image),error,1024)==1,error.value.decode())
                pixels=np.frombuffer(C.string_at(image.pixels,image.pixels_len),dtype=np.uint8).reshape(image.height,image.width,4).copy()
                icc=C.string_at(image.icc,image.icc_len); raw=C.string_at(image.exif,image.exif_len)
                if raw: raw=raw[4+int.from_bytes(raw[:4],'big'):]
                exif=Image.Exif()
                if raw:exif.load(raw)
                return pixels,icc,exif
            finally:self.lib.rh_free_image(C.byref(image));self.lib.rh_close(document)
        with Image.open(path) as image:
            image.load(); pixels=np.array(image.convert('RGBA')); icc=image.info.get('icc_profile',b''); exif=image.getexif()
        return pixels,icc,exif


def metadata(w,h,orientation=1,large=False):
    exif=Image.Exif(); exif[256]=w; exif[257]=h; exif[274]=orientation; exif[315]='Roshan synthetic QA'
    if large:exif[50740]=b'PRIVATE TEST DATA\0'+b'X'*70000
    return exif.tobytes()
def linear_flatten(rgba):
    values=rgba[...,:3].astype(np.float64)/255.0; alpha=rgba[...,3:4].astype(np.float64)/255.0
    linear=np.where(values<=0.04045, values/12.92, ((values+0.055)/1.055)**2.4)
    mixed=linear*alpha+1-alpha
    encoded=np.where(mixed<=0.0031308,12.92*mixed,1.055*mixed**(1/2.4)-0.055)
    return np.rint(np.clip(encoded,0,1)*255).astype(np.uint8)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli',type=Path,required=True);parser.add_argument('--native-dir',type=Path,required=True)
    parser.add_argument('--work-dir',type=Path,required=True);parser.add_argument('--report',type=Path,required=True)
    parser.add_argument('--cxx',default='g++')
    parser.add_argument('--wine',type=Path,help='Run a Windows CLI under this Wine executable; host paths use the Z: mapping')
    parser.add_argument('--wine-env',type=Path,help='JSON environment variables for an existing Wine prefix')
    args=parser.parse_args()
    root=Path(__file__).resolve().parent.parent;work=args.work_dir.resolve();work.mkdir(parents=True,exist_ok=True)
    work=Path(tempfile.mkdtemp(prefix='image-matrix-',dir=work))
    args.cli=args.cli.resolve(); inspector=Inspector(root,args.native_dir.resolve(),work,args.cxx)
    application_hash=sha(args.cli)
    cli_env=os.environ.copy()
    if args.wine_env:cli_env.update(json.loads(args.wine_env.read_text()))
    if args.wine:cli_env.pop('DISPLAY',None)
    def cli_path(path):
        value=str(Path(path).resolve())
        return 'Z:'+value.replace('/','\\') if args.wine else value
    def output_path(value):
        if not args.wine:return Path(value)
        if value.startswith('\\\\?\\'):value=value[4:]
        require(value[:2].lower()=='z:',f'Unexpected Wine output drive: {value}')
        return Path(value[2:].replace('\\','/'))
    def execute(arguments):
        command=([str(args.wine)] if args.wine else [])+[str(args.cli)]+arguments
        return subprocess.run(command,env=cli_env,text=True,encoding='utf-8',errors='replace',stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=120)
    results=[]; runs=[]; sequence=0
    def convert(path,target,label,strip=False,expect=True,directory=None):
        nonlocal sequence
        sequence+=1;directory=directory or work/label;directory.mkdir(parents=True,exist_ok=True)
        report=work/f'cli-report-{sequence:03d}.json'
        command=['--convert','--to',target,'--output',cli_path(directory),'--report',cli_path(report)]
        if strip:command+=['--strip-exif']
        command+=[cli_path(path)]
        run=execute(command)
        require(report.is_file(),f'{label}: report missing: {run.stderr}')
        record=json.loads(report.read_text())['files'][0];runs.append({'case':label,'exit':run.returncode,'record':record})
        require((run.returncode==0)==expect,f'{label}: unexpected exit {run.returncode}: {run.stderr}')
        if not expect:
            require(record['status']=='error',f'{label}: error missing');return record
        require(record['status']=='ok' and len(record['outputs'])==1,f'{label}: unexpected output count')
        return output_path(record['outputs'][0]['path']),record['outputs'][0]
    def passed(name,**details):results.append({'case':name,'result':'passed',**details});print('PASS',name,flush=True)

    # RGB values under alpha=0 are deliberately nonzero; losslessness includes them.
    w,h=73,67;y,x=np.mgrid[:h,:w];rgba=np.empty((h,w,4),np.uint8)
    rgba[...,0]=(x*37+y*11)&255;rgba[...,1]=(x*3+y*71+13)&255;rgba[...,2]=(x*139+y*7+191)&255
    rgba[...,3]=np.array([0,64,128,192,255],np.uint8)[(x//9+y//11)%5]
    srgb=ImageCms.ImageCmsProfile(ImageCms.createProfile('sRGB')).tobytes()
    source=work/'synthetic-rgba.png';save_png(source,rgba,srgb,metadata(w,h))
    seeds={'png':source}
    for target in ['heic','jpg','webp']:seeds[target]=convert(source,target,'seed-'+target)[0]
    for src,path in seeds.items():
        original,profile,_=inspector.read(path)
        for dst in ['heic','png','jpg','webp']:
            if src==dst:continue
            output,info=convert(path,dst,f'matrix-{src}-to-{dst}')
            actual,icc,exif=inspector.read(output)
            require(actual.shape==original.shape,f'{src}->{dst}: dimensions changed')
            require(icc==profile,f'{src}->{dst}: native RGB ICC changed')
            require(exif.get(274)==1 and exif.get(256)==w and exif.get(257)==h and exif.get(315)=='Roshan synthetic QA',f'{src}->{dst}: EXIF mismatch')
            maximum=None
            if dst=='jpg':
                expected=linear_flatten(original);delta=np.abs(actual[...,:3].astype(int)-expected.astype(int));maximum=int(delta.max())
                require(maximum<=7 and float(delta.mean())<1.3,f'{src}->jpg: excessive error {maximum}/{delta.mean()}')
                with Image.open(output) as jpg:
                    require(all(layer[1:3]==(1,1) for layer in jpg.layer),'JPEG is not 4:4:4')
                    require(all(v==1 for table in jpg.quantization.values() for v in table),'JPEG quality is not 100')
                require(np.all(actual[...,3]==255),'JPEG retained alpha unexpectedly')
            else:
                delta=np.abs(actual.astype(int)-original.astype(int));maximum=int(delta.max())
                require(maximum <= (2 if src=='jpg' else 0),f'{src}->{dst}: lossless samples differ by {maximum}')
            passed(f'{src}->{dst}',width=w,height=h,icc_bytes=len(icc),alpha=dst!='jpg' and src!='jpg',max_sample_difference=maximum,warnings=info['warnings'])
    for src,path in seeds.items():
        output,_=convert(path,src,'copy-'+src)
        require(path.read_bytes()==output.read_bytes(),f'{src}: same-format copy changed bytes')
        passed('same-format byte copy '+src,sha256=sha(output))

    # All EXIF orientations, checked against an independent Pillow transform.
    orient=rgba[:13,:19].copy()
    for kind in ['png','jpg','webp']:
        for orientation in range(1,9):
            path=work/f'orientation-{kind}-{orientation}.{kind}'
            if kind=='png':save_png(path,orient,srgb,metadata(19,13,orientation))
            else:
                image=Image.fromarray(orient)
                if kind=='jpg':image=image.convert('RGB')
                image.save(path,format='JPEG' if kind=='jpg' else 'WEBP',quality=100,subsampling=0,lossless=True,exact=True,icc_profile=srgb,exif=metadata(19,13,orientation))
            with Image.open(path) as im: expected=np.array(ImageOps.exif_transpose(im).convert('RGBA'))
            output,_=convert(path,'heic' if kind=='png' else 'png',f'orientation-out-{kind}-{orientation}')
            actual,icc,exif=inspector.read(output)
            require(actual.shape==expected.shape,f'{kind} orientation {orientation}: wrong geometry')
            require(np.abs(actual.astype(int)-expected.astype(int)).max() <= (2 if kind=='jpg' else 0),f'{kind} orientation {orientation}: pixels wrong')
            require(exif.get(274)==1 and exif.get(256)==expected.shape[1] and exif.get(257)==expected.shape[0],'Orientation tags not normalized')
    passed('all 8 EXIF orientations in PNG/JPG/WebP',conversions=24)

    # Preserve real 16-bit PNG samples on re-encode; other targets warn about 8-bit conversion.
    y16,x16=np.mgrid[:17,:31];hi=np.empty((17,31,4),np.uint16)
    hi[...,0]=(x16*2003+y16*11)&65535;hi[...,1]=(x16*257+y16*3107)&65535;hi[...,2]=(65535-x16*761-y16*197)&65535;hi[...,3]=(x16*1019+y16*911)&65535
    high=work/'high-depth.png';save_png(high,hi,srgb,metadata(31,17))
    kept,_=convert(high,'png','high-depth-png',strip=True)
    require(np.array_equal(read_png_samples(kept),hi),'16-bit PNG precision changed')
    require(inspector.read(kept)[1]==srgb and not inspector.read(kept)[2],'PNG strip-EXIF dropped ICC or retained EXIF')
    for target in ['heic','webp','jpg']:
        out,info=convert(high,target,'high-depth-'+target)
        require(any('8-bit' in s for s in info['warnings']),f'No high-precision warning for {target}')
        require(inspector.read(out)[0].shape==(17,31,4),'High-depth conversion dimensions wrong')
    passed('16-bit PNG preservation and explicit 8-bit conversion warnings')

    # Large metadata fits PNG/WebP/HEIC, but JPEG has its own smaller APP1 limit.
    large=work/'large-exif.png';save_png(large,rgba,srgb,metadata(w,h,large=True))
    for target in ['heic','webp','png','jpg']:
        out,info=convert(large,target,'large-exif-'+target)
        _,_,exif=inspector.read(out)
        if target=='jpg':require(any('EXIF' in s and 'limit' in s for s in info['warnings']) and not exif,'Large JPEG EXIF not safely omitted')
        else:require(exif.get(50740)==b'PRIVATE TEST DATA\0'+b'X'*70000,f'Large {target} EXIF lost')
    passed('target-specific EXIF limits preserve 70 KiB metadata outside JPEG')

    # New PNG color precedence: cICP wins even when a contradictory legacy ICC/gAMA exists.
    for codes in [(12,13,0,1),(9,14,0,1),(1,13,0,1)]:
        name='cicp-'+'-'.join(map(str,codes));path=work/(name+'.png')
        save_png(path,rgba,srgb,metadata(w,h),[(b'cICP',bytes(codes)),(b'gAMA',struct.pack('>I',100000))])
        out,_=convert(path,'webp',name)
        actual,profile,_=inspector.read(out)
        require(np.array_equal(actual,rgba),'cICP conversion changed encoded samples')
        require(generated_profile_equal(profile,inspector.profile(codes)),'cICP did not override the conflicting ICC/gamma')
    passed('PNG full-range SDR cICP: Display-P3, BT.2020, sRGB; correct precedence')
    for codes in [(2,2,0,1),(1,1,0,0),(1,13,1,1),(9,16,0,1),(9,18,0,1)]:
        name='unsupported-cicp-'+'-'.join(map(str,codes));path=work/(name+'.png')
        save_png(path,rgba,srgb,None,[(b'cICP',bytes(codes))]);convert(path,'webp',name,expect=False)
        if codes[1] in (16,18):
            copied,_=convert(path,'png',name+'-copy');require(path.read_bytes()==copied.read_bytes(),'Original HDR PNG copy changed')
    passed('unsupported/unknown, narrow-range, non-RGB and HDR cICP fail without false color; HDR copy exact')
    duplicate=work/'duplicate-cicp.png';save_png(duplicate,rgba,srgb,None,[(b'cICP',bytes([12,13,0,1])),(b'cICP',bytes([1,13,0,1]))])
    convert(duplicate,'webp','duplicate-cicp',expect=False)
    bad_crc=work/'bad-cicp-crc.png';save_png(bad_crc,rgba,srgb,None,[(b'cICP',bytes([12,13,0,1]))])
    damaged=bytearray(bad_crc.read_bytes());location=damaged.index(b'cICP');damaged[location+8]^=1;bad_crc.write_bytes(damaged)
    convert(bad_crc,'webp','bad-cicp-crc',expect=False)
    passed('malformed/duplicate PNG cICP is rejected')
    black=np.zeros((32,32,4),np.uint8);black[...,3]=128
    linear=work/'linear-gamma.png';save_png(linear,black,None,None,[(b'gAMA',struct.pack('>I',100000))])
    output,_=convert(linear,'jpg','linear-gamma-jpg');values=inspector.read(output)[0]
    require(np.all((values[...,:3]>=126)&(values[...,:3]<=128)),'PNG gAMA alpha composite used wrong transfer')
    passed('PNG gAMA=1.0 alpha composites in its native linear color coordinates')

    # Existing data and failures must not be overwritten, and a failed batch entry must not stop the next file.
    destination=work/'no-overwrite';destination.mkdir(exist_ok=True);existing=destination/'synthetic-rgba.jpg';existing.write_bytes(b'unchanged sentinel')
    output,_=convert(source,'jpg','no-overwrite',directory=destination)
    require(existing.read_bytes()==b'unchanged sentinel' and output!=existing,'Existing destination overwritten')
    corrupt=work/'corrupted.heic';corrupt.write_bytes(b'not a valid HEIC file')
    convert(corrupt,'png','corrupt-input',expect=False)
    unicode=work/'تصویر شفاف ۱۲.png';unicode.write_bytes(source.read_bytes());convert(unicode,'webp','unicode-path')
    sequence+=1;batch_report=work/f'cli-report-{sequence:03d}.json';batch_out=work/'batch-after-error'
    batch=execute(['--convert','--to','png','--output',cli_path(batch_out),'--report',cli_path(batch_report),cli_path(corrupt),cli_path(seeds['heic'])])
    records=json.loads(batch_report.read_text())['files'];runs.append({'case':'batch-after-error','exit':batch.returncode,'record':records})
    require(batch.returncode!=0 and [r['status'] for r in records]==['error','ok'],'A corrupt batch entry blocked the valid image')
    require(existing.read_bytes()==b'unchanged sentinel','Existing output changed later')
    passed('no-overwrite publication, corrupt-input error, batch continuation, Unicode path')

    require(sha(args.cli)==application_hash,'The tested executable changed during the matrix')
    report={'result':'passed','application':str(args.cli),'application_sha256':application_hash,
            'target':'windows-under-wine' if args.wine else 'linux-native',
            'directed_format_pairs':12,'same_format_copies':4,'orientation_conversions':24,
            'script_sha256':sha(Path(__file__)),'native_bridge_sha256':sha(root/'native/bridge.cpp'),
            'source_sha256':{name:sha(root/'src'/name) for name in ['image_engine.rs','color.rs','metadata.rs','safe_output.rs','engine.rs']},
            'cases':results,'cli_runs':runs,'fixture_policy':'All fixtures are synthetic and generated by this script. No user photos, external downloads or user files are used.',
            'references':['https://www.w3.org/TR/png-3/#cICP-chunk','https://github.com/strukturag/libheif/releases/tag/v1.23.6','https://github.com/Multicorewareinc/x265/releases/tag/4.3'],
            'limits':'JPEG is lossy despite quality100/4:4:4. Comparison allows at most two sample levels between independent JPEG decoders. HEIC output is lossless for the supplied RGB8/RGBA8 pipeline data, not source HDR/high precision. This report exercises the actual Rust CLI and linked codecs; Windows GUI QA is separate.'}
    args.report.parent.mkdir(parents=True,exist_ok=True);args.report.write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n')
    print(f'PASS: {len(results)} groups, {sequence} CLI runs; report {args.report}')


if __name__=='__main__':main()
