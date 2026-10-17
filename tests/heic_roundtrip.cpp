// Synthetic, redistributable tests. No external photographs are required.
#include "../native/bridge.h"
#include <libheif/heif.h>
#include <lcms2.h>
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

static void require(bool condition, const char* message) {
  if (!condition) throw std::runtime_error(message);
}
struct Buffer {
  RhBuffer value{};
  ~Buffer() { rh_free_buffer(&value); }
};
struct Image {
  RhImage value{};
  ~Image() { rh_free_image(&value); }
};
struct Document {
  void* value = nullptr;
  ~Document() { rh_close(value); }
};
static void u16(std::vector<uint8_t>& out, size_t i, uint16_t v) {
  out[i] = static_cast<uint8_t>(v); out[i + 1] = static_cast<uint8_t>(v >> 8);
}
static void u32(std::vector<uint8_t>& out, size_t i, uint32_t v) {
  u16(out, i, static_cast<uint16_t>(v)); u16(out, i + 2, static_cast<uint16_t>(v >> 16));
}
static std::vector<uint8_t> exif(uint32_t w, uint32_t h) {
  std::vector<uint8_t> bytes(50, 0);
  bytes[0] = bytes[1] = 'I'; u16(bytes, 2, 42); u32(bytes, 4, 8); u16(bytes, 8, 3);
  u16(bytes, 10, 256); u16(bytes, 12, 4); u32(bytes, 14, 1); u32(bytes, 18, w);
  u16(bytes, 22, 257); u16(bytes, 24, 4); u32(bytes, 26, 1); u32(bytes, 30, h);
  u16(bytes, 34, 274); u16(bytes, 36, 3); u32(bytes, 38, 1); u16(bytes, 42, 1);
  return bytes;
}
static std::vector<uint8_t> p3_profile() {
  cmsCIExyY white{0.3127, 0.3290, 1.0};
  cmsCIExyYTRIPLE primaries{{0.680,0.320,1.0},{0.265,0.690,1.0},{0.150,0.060,1.0}};
  cmsToneCurve* curve = cmsBuildGamma(nullptr, 2.2);
  require(curve != nullptr, "Cannot create test curve");
  cmsToneCurve* curves[3]{curve,curve,curve};
  cmsHPROFILE profile = cmsCreateRGBProfile(&white, &primaries, curves);
  cmsFreeToneCurve(curve);
  require(profile != nullptr, "Cannot create test P3 ICC");
  cmsUInt32Number length = 0;
  require(cmsSaveProfileToMem(profile, nullptr, &length) != 0, "Cannot size test ICC");
  std::vector<uint8_t> bytes(length);
  require(cmsSaveProfileToMem(profile, bytes.data(), &length) != 0, "Cannot save test ICC");
  cmsCloseProfile(profile);
  return bytes;
}

static void roundtrip(uint32_t w, uint32_t h, uint32_t channels,
                       const uint8_t* profile, size_t profile_len, bool print = true) {
  char error[1024]{};
  std::vector<uint8_t> pixels(size_t(w) * h * channels);
  for (uint32_t y = 0; y < h; ++y) for (uint32_t x = 0; x < w; ++x) {
    size_t i = (size_t(y) * w + x) * channels;
    pixels[i] = static_cast<uint8_t>((x * 37 + y * 11) & 255);
    pixels[i + 1] = static_cast<uint8_t>((x * 3 + y * 71 + 13) & 255);
    pixels[i + 2] = static_cast<uint8_t>((x * 139 + y * 7 + 191) & 255);
    if (channels == 4) pixels[i + 3] = static_cast<uint8_t>((x + y * 3) & 255);
  }
  std::vector<uint8_t> metadata = exif(w, h);
  Buffer encoded;
  require(rh_encode_heic(pixels.data(), pixels.size(), w, h, channels, profile, profile_len,
                        metadata.data(), metadata.size(), &encoded.value, error, sizeof(error)) == 1, error);
  require((encoded.value.flags & RH_HEIC_LOSSLESS) != 0, "HEIC output lacks verified lossless flag");
  require(bool(encoded.value.flags & RH_ALPHA_PRESERVED) == (channels == 4), "Encoded alpha flag mismatch");
  require(bool(encoded.value.flags & RH_COLOR_ASSUMED) == (profile_len == 0), "Encoded color assumption flag mismatch");
  require(encoded.value.len > 20, "Encoded output empty");
  Document document;
  document.value = rh_open(encoded.value.data, encoded.value.len, error, sizeof(error));
  require(document.value != nullptr, error);
  require(rh_count(document.value) == 1, "Alpha incorrectly exposed as a top-level image");
  Image decoded;
  require(rh_decode_rgba(document.value, 0, &decoded.value, error, sizeof(error)) == 1, error);
  require(decoded.value.width == w && decoded.value.height == h, "Original/odd dimensions changed");
  require(decoded.value.pixels_len == size_t(w) * h * 4, "Invalid decoded RGBA8 layout");
  require((decoded.value.flags & (RH_ALPHA_ON_WHITE | RH_HIGH_DEPTH_TO_8 | RH_HDR_TONE_MAPPED | RH_ICC_OMITTED)) == 0,
          "Lossless roundtrip produced unexpected precision/color conversion");
  require(bool(decoded.value.flags & RH_ALPHA_PRESERVED) == (channels == 4), "Decoded alpha flag mismatch");
  size_t differences = 0;
  for (size_t p = 0; p < size_t(w) * h; ++p) {
    for (uint32_t c = 0; c < channels; ++c)
      differences += pixels[p * channels + c] != decoded.value.pixels[p * 4 + c];
    if (channels == 3) require(decoded.value.pixels[p * 4 + 3] == 255, "Opaque source gained transparency");
  }
  if (differences) {
    std::fprintf(stderr, "%ux%u channels=%u differences=%zu\n", w, h, channels, differences);
    throw std::runtime_error("The HEIC roundtrip changed source 8-bit pixel samples");
  }
  if (profile_len) {
    require(decoded.value.icc_len == profile_len, "ICC length changed");
    require(std::memcmp(decoded.value.icc, profile, profile_len) == 0, "ICC content changed");
  } else require(decoded.value.icc_len >= 128, "Assumed sRGB profile is missing");
  require(decoded.value.exif_len >= 4, "EXIF is missing");
  const uint8_t* ex = decoded.value.exif;
  size_t offset = 4 + (size_t(ex[0]) << 24) + (size_t(ex[1]) << 16) + (size_t(ex[2]) << 8) + ex[3];
  require(offset <= decoded.value.exif_len && decoded.value.exif_len - offset == metadata.size(), "EXIF size/offset changed");
  require(std::memcmp(ex + offset, metadata.data(), metadata.size()) == 0, "Normalized EXIF changed");

  // Inspect the stored identity matrix and native plane shape independently.
  heif_context* context = heif_context_alloc();
  require(context != nullptr, "Cannot allocate HEIF inspection context");
  heif_error status = heif_context_read_from_memory_without_copy(context, encoded.value.data, encoded.value.len, nullptr);
  require(status.code == heif_error_Ok, status.message);
  heif_image_handle* handle = nullptr;
  status = heif_context_get_primary_image_handle(context, &handle);
  require(status.code == heif_error_Ok, status.message);
  heif_color_profile_nclx* nclx = nullptr;
  status = heif_image_handle_get_nclx_color_profile(handle, &nclx);
  require(status.code == heif_error_Ok && nclx, "HEIC NCLX metadata missing");
  require(nclx->matrix_coefficients == 0 && nclx->full_range_flag != 0, "HEIC is not full-range RGB/GBR identity");
  heif_nclx_color_profile_free(nclx);
  require(heif_image_handle_get_luma_bits_per_pixel(handle) == 8, "HEIC component depth is not 8 bit");
  heif_image* native = nullptr;
  status = heif_decode_image(handle, &native, heif_colorspace_YCbCr, heif_chroma_444, nullptr);
  require(status.code == heif_error_Ok, status.message);
  require(heif_image_get_width(native, heif_channel_Y) == heif_image_get_width(native, heif_channel_Cb) &&
          heif_image_get_height(native, heif_channel_Y) == heif_image_get_height(native, heif_channel_Cb),
          "Encoded chroma plane is subsampled");
  heif_image_release(native);
  heif_image_handle_release(handle);
  heif_context_free(context);
  if (print) std::printf("PASS roundtrip %ux%u channels=%u icc=%zu heic=%zu flags=%u exact_samples=yes exif=unchanged matrix=GBR444\n",
                          w, h, channels, decoded.value.icc_len, encoded.value.len, encoded.value.flags);
}

int main() {
  try {
    char error[1024]{};
    Buffer srgb, same;
    require(rh_srgb_profile(&srgb.value, error, sizeof(error)) == 1, error);
    require(rh_srgb_profile(&same.value, error, sizeof(error)) == 1, error);
    require(srgb.value.len == same.value.len && !std::memcmp(srgb.value.data, same.value.data, srgb.value.len),
            "Canonical sRGB serialization is not deterministic");
    const auto p3 = p3_profile();
    roundtrip(1, 1, 3, srgb.value.data, srgb.value.len);
    roundtrip(1, 1, 4, srgb.value.data, srgb.value.len);
    roundtrip(2, 3, 4, nullptr, 0);
    roundtrip(29, 100, 4, p3.data(), p3.size());
    roundtrip(128, 128, 3, srgb.value.data, srgb.value.len);
    roundtrip(257, 193, 4, p3.data(), p3.size());
    roundtrip(73, 67, 3, nullptr, 0);

    const uint8_t rgba[]{255,0,0,128, 17,91,133,0, 37,87,123,255};
    Buffer flattened;
    require(rh_flatten_rgba(rgba, sizeof(rgba), 3, 1, srgb.value.data, srgb.value.len,
                           &flattened.value, error, sizeof(error)) == 1, error);
    require(flattened.value.len == 9 && (flattened.value.flags & RH_ALPHA_ON_WHITE), "Flattened buffer/flag invalid");
    require(flattened.value.data[0] == 255 && flattened.value.data[1] >= 187 && flattened.value.data[1] <= 188 &&
            flattened.value.data[2] >= 187 && flattened.value.data[2] <= 188, "Alpha was not composited in linear light");
    require(flattened.value.data[3] == 255 && flattened.value.data[4] == 255 && flattened.value.data[5] == 255,
            "Transparent pixels were not composited to white");
    require(flattened.value.data[6] == 37 && flattened.value.data[7] == 87 && flattened.value.data[8] == 123,
            "Opaque source samples changed during flattening");
    Buffer rejected;
    const uint8_t bad_icc[]{0,1,2,3};
    require(rh_flatten_rgba(rgba, sizeof(rgba), 3, 1, bad_icc, sizeof(bad_icc), &rejected.value, error, sizeof(error)) == 0 &&
            rejected.value.data == nullptr && rejected.value.len == 0, "Invalid ICC was silently accepted");
    require(rh_encode_heic(rgba, 1, 3, 1, 4, nullptr, 0, nullptr, 0, &rejected.value, error, sizeof(error)) == 0 &&
            rejected.value.data == nullptr && rejected.value.len == 0, "Invalid pixel length was accepted");
    std::puts("PASS linear-alpha flattening, opaque samples, invalid-ICC/buffer rejection, deterministic ICC");

    // Exercise independently allocated codec contexts, as a background batch
    // implementation may run jobs concurrently with varying image dimensions.
    std::string failures[2];
    std::thread workers[2];
    for (int i = 0; i < 2; ++i) workers[i] = std::thread([&, i]() {
      try { roundtrip(i ? 257 : 17, i ? 193 : 19, 4, srgb.value.data, srgb.value.len, false); }
      catch (const std::exception& e) { failures[i] = e.what(); }
    });
    for (auto& worker : workers) worker.join();
    for (const auto& failure : failures) require(failure.empty(), failure.c_str());
    std::puts("PASS concurrent independent HEIC encoders and RGBA decoders");
    std::puts("PASS HEIC V2: 9 synthetic RGB/RGBA lossless roundtrips including alpha=0 hidden color, odd/tiny sizes, ICC and EXIF");
    return 0;
  } catch (const std::exception& error) {
    std::fprintf(stderr, "FAIL: %s\n", error.what());
    return 1;
  }
}
