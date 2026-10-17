#ifndef ROSHAN_HEIC_BRIDGE_H
#define ROSHAN_HEIC_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum RhFlags {
  RH_HDR_TONE_MAPPED = 1u,
  RH_ALPHA_ON_WHITE = 2u,
  RH_HIGH_DEPTH_TO_8 = 4u,
  RH_COLOR_ASSUMED = 8u,
  RH_DECODER_WARNING = 16u,
  RH_HEIF_ORIENTATION = 32u,
  RH_EXIF_OMITTED = 64u,
  RH_ICC_OMITTED = 128u,
  RH_GRAY_PROFILE_TO_SRGB = 256u,
  RH_ALPHA_PRESERVED = 512u,
  RH_HEIC_LOSSLESS = 1024u
};

typedef struct RhImage {
  uint32_t width;
  uint32_t height;
  uint8_t* pixels;       /* RGB8 for rh_decode, RGBA8 for rh_decode_rgba. */
  size_t pixels_len;
  uint8_t* icc;          /* Profile matching the returned RGB pixels. */
  size_t icc_len;
  uint8_t* exif;         /* Original HEIF Exif block, including 4-byte BE offset. */
  size_t exif_len;
  uint32_t flags;
} RhImage;

typedef struct RhBuffer {
  uint8_t* data;
  size_t len;
  uint32_t flags;
} RhBuffer;

/* The input memory must stay alive and unchanged until rh_close().
 * The returned document is not safe for concurrent calls on that same document.
 * All top-level still images are exposed, primary first; thumbnails/auxiliaries
 * are not independently exported. Geometric HEIF transforms are applied.
 * rh_decode returns 1 on success, 0 on failure. Pass a fresh/zeroed RhImage;
 * rh_free_image releases returned allocations and resets every field to zero.
 * RH_HEIF_ORIENTATION means an explicit irot or imir property was present,
 * including an explicit identity rotation. EXIF tags are not interpreted here.
 * All strings written to err are bounded, NUL-terminated UTF-8 (English).
 * No C++ exception crosses this interface.
 */
void* rh_open(const uint8_t* data, size_t len, char* err, size_t errlen);
int rh_count(void* document);
int rh_decode(void* document, int index, RhImage* out, char* err, size_t errlen);
/* Straight (unassociated) RGBA8, even for images without alpha. Original
 * transparency is retained and flagged RH_ALPHA_PRESERVED when present.
 * The same SDR colour/HDR-rendering policy as rh_decode applies. */
int rh_decode_rgba(void* document, int index, RhImage* out, char* err, size_t errlen);
void rh_free_image(RhImage* image);
void rh_close(void* document);
const char* rh_version(void);

/* All buffer APIs return 1 on success and 0 on failure. Pass a fresh/zeroed
 * RhBuffer and free it using rh_free_buffer, including after a failed call.
 * Pixel lengths must equal width * height * channels; rows have no padding.
 * HEIC encoding uses x265 lossless 8-bit 4:4:4 with RGB/GBR identity matrix,
 * avoiding chroma subsampling and colour-matrix rounding. Alpha is encoded
 * as a lossless auxiliary image. This preserves the supplied 8-bit samples;
 * it cannot reconstruct source precision or HDR already rendered to SDR.
 * A valid RGB ICC is embedded unchanged. Missing ICC is tagged sRGB and
 * flagged RH_COLOR_ASSUMED; invalid/unsupported ICC fails explicitly.
 * exif_tiff must be normalized TIFF data (without an Exif prefix or HEIF
 * offset), with Orientation=1 and dimensions already matching the pixels.
 * This interface accepts single still images; sequence packaging is separate.
 */
int rh_encode_heic(const uint8_t* pixels, size_t pixels_len,
                   uint32_t width, uint32_t height, uint32_t channels,
                   const uint8_t* icc, size_t icc_len,
                   const uint8_t* exif_tiff, size_t exif_len,
                   RhBuffer* out, char* err, size_t errlen);
void rh_free_buffer(RhBuffer* buffer);

/* Composite straight RGBA8 over white in linear XYZ through a valid RGB ICC,
 * returning packed RGB8 in the SAME colour coordinates. Missing ICC assumes
 * sRGB and sets RH_COLOR_ASSUMED; invalid/unsupported ICC fails explicitly.
 * These helpers do not change or return the caller's ICC. */
int rh_flatten_rgba(const uint8_t* rgba, size_t rgba_len,
                    uint32_t width, uint32_t height,
                    const uint8_t* icc, size_t icc_len,
                    RhBuffer* out, char* err, size_t errlen);
/* Returns a serialized canonical sRGB ICC profile with a fixed timestamp. */
int rh_srgb_profile(RhBuffer* out, char* err, size_t errlen);
/* Build an ICC matching supported SDR H.273/cICP primaries and transfer.
 * Accepts only RGB identity matrix (0) and full range (1). Unknown/reserved
 * color codes, PQ/HLG and narrow range fail explicitly without assumptions.
 * This helper creates a profile; it does not transform pixel samples. */
int rh_nclx_profile(uint32_t primaries, uint32_t transfer, uint32_t matrix,
                     uint32_t full_range, RhBuffer* out, char* err, size_t errlen);

/* Colour policy:
 * SDR pixels retain their native RGB gamut and valid RGB ICC profile. NCLX-only
 * files receive an ICC built from their supported primaries/transfer function.
 * Unknown colour data is explicitly flagged, with sRGB assumptions as needed.
 * Alpha is unpremultiplied when signalled. rh_decode composites over white
 * in linear XYZ through Little CMS; rh_decode_rgba preserves straight alpha.
 * Grayscale ICC is converted to tagged sRGB.
 * PQ/HLG: decode at source precision, apply the PQ EOTF (10000-nit scale) or
 * HLG inverse OETF plus 1000-nit, gamma-1.2 OOTF; Bradford-adapt the source
 * primaries to linear sRGB; apply image-global, luminance-preserving modified
 * Reinhard, with 203-nit reference and measured image peak (at least 203 nits).
 * Finally, composite white when requested, clip out-of-sRGB gamut, encode
 * sRGB, and round to 8 bits. This is a deterministic SDR rendering, not
 * preservation of HDR.
 */

#ifdef __cplusplus
}
#endif
#endif
