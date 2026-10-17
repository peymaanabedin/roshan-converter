#include "bridge.h"

#ifndef LIBHEIF_STATIC_BUILD
#define LIBHEIF_STATIC_BUILD
#endif
#include <libheif/heif.h>
#include <libheif/heif_properties.h>
#include <lcms2.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <memory>
#include <new>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
constexpr uint64_t kMaxPixels = 200000000;
constexpr uint32_t kMaxDimension = 65535;
constexpr size_t kMaxProfile = 4u * 1024u * 1024u;
constexpr size_t kMaxExif = 16u * 1024u * 1024u;
constexpr size_t kMaxInput = size_t{2} * 1024u * 1024u * 1024u;
constexpr double kReferenceNits = 203.0;

void error_text(char* dst, size_t capacity, const char* message) noexcept {
  if (!dst || !capacity) return;
  const char* text = message ? message : "Unknown conversion error.";
  const size_t n = std::min(capacity - 1, std::strlen(text));
  std::memcpy(dst, text, n);
  dst[n] = '\0';
}

void check(heif_error e) {
  if (e.code != heif_error_Ok) {
    // Copy while the libheif object still owns the message.
    throw std::runtime_error(e.message ? e.message : "HEIC decoding failed.");
  }
}

struct Document {
  bool initialized = false;
  heif_context* context = nullptr;
  std::vector<heif_item_id> ids;
  ~Document() noexcept {
    if (context) heif_context_free(context);
    if (initialized) heif_deinit();
  }
};
using Handle = std::unique_ptr<heif_image_handle, decltype(&heif_image_handle_release)>;
using Decoded = std::unique_ptr<heif_image, decltype(&heif_image_release)>;
using Options = std::unique_ptr<heif_decoding_options, decltype(&heif_decoding_options_free)>;
using EncodeOptions = std::unique_ptr<heif_encoding_options, decltype(&heif_encoding_options_free)>;
using Encoder = std::unique_ptr<heif_encoder, decltype(&heif_encoder_release)>;
using Nclx = std::unique_ptr<heif_color_profile_nclx, decltype(&heif_nclx_color_profile_free)>;

struct Profile {
  cmsHPROFILE value = nullptr;
  Profile() = default;
  explicit Profile(cmsHPROFILE p) : value(p) {}
  Profile(const Profile&) = delete;
  Profile& operator=(const Profile&) = delete;
  Profile(Profile&& other) noexcept : value(other.value) { other.value = nullptr; }
  Profile& operator=(Profile&& other) noexcept {
    if (this != &other) {
      if (value) cmsCloseProfile(value);
      value = other.value;
      other.value = nullptr;
    }
    return *this;
  }
  ~Profile() { if (value) cmsCloseProfile(value); }
};
struct Transform {
  cmsHTRANSFORM value = nullptr;
  Transform() = default;
  Transform(const Transform&) = delete;
  Transform& operator=(const Transform&) = delete;
  ~Transform() { if (value) cmsDeleteTransform(value); }
};
struct ImageGuard {
  RhImage image{};
  ~ImageGuard() { rh_free_image(&image); }
};
struct BufferGuard {
  RhBuffer buffer{};
  ~BufferGuard() { rh_free_buffer(&buffer); }
};

double unit(double x) {
  return std::isfinite(x) ? std::clamp(x, 0.0, 1.0) : 0.0;
}
uint8_t quantize(double x) { return static_cast<uint8_t>(std::lround(unit(x) * 255.0)); }
double srgb_linear(double x) {
  x = std::max(0.0, x);
  return x <= 0.04045 ? x / 12.92 : std::pow((x + 0.055) / 1.055, 2.4);
}
double srgb_encode(double x) {
  x = std::max(0.0, x);
  return x <= 0.0031308 ? 12.92 * x : 1.055 * std::pow(x, 1.0 / 2.4) - 0.055;
}

struct Primaries {
  cmsCIExyY white;
  cmsCIExyYTRIPLE rgb;
};
Primaries primaries_for(int code, uint32_t& flags, bool hdr = false) {
  Primaries p{{0.3127, 0.3290, 1.0}, {{0.64, 0.33, 1.0}, {0.30, 0.60, 1.0}, {0.15, 0.06, 1.0}}};
  switch (code) {
    case 1: break;
    case 4:
      p = {{0.31006, 0.31616, 1.0}, {{0.67, 0.33, 1.0}, {0.21, 0.71, 1.0}, {0.14, 0.08, 1.0}}};
      break;
    case 5:
      p.rgb = {{0.64, 0.33, 1.0}, {0.29, 0.60, 1.0}, {0.15, 0.06, 1.0}};
      break;
    case 6: case 7:
      p.rgb = {{0.630, 0.340, 1.0}, {0.310, 0.595, 1.0}, {0.155, 0.070, 1.0}};
      break;
    case 9:
      p.rgb = {{0.708, 0.292, 1.0}, {0.170, 0.797, 1.0}, {0.131, 0.046, 1.0}};
      break;
    case 11: case 12:
      p.rgb = {{0.680, 0.320, 1.0}, {0.265, 0.690, 1.0}, {0.150, 0.060, 1.0}};
      if (code == 11) p.white = {0.314, 0.351, 1.0};
      break;
    case 22:
      p.rgb = {{0.630, 0.340, 1.0}, {0.295, 0.605, 1.0}, {0.155, 0.077, 1.0}};
      break;
    default:
      flags |= RH_COLOR_ASSUMED;
      if (hdr) p.rgb = {{0.708, 0.292, 1.0}, {0.170, 0.797, 1.0}, {0.131, 0.046, 1.0}};
      break;
  }
  return p;
}

cmsToneCurve* curve_for(int transfer, uint32_t& flags) {
  double p[5] = {2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045};
  switch (transfer) {
    case 1: case 6: case 14:
      p[0] = 1.0 / 0.45; p[1] = 1.0 / 1.099; p[2] = 0.099 / 1.099; p[3] = 1.0 / 4.5; p[4] = 0.081;
      break;
    case 15:
      p[0] = 1.0 / 0.45; p[1] = 1.0 / 1.0993; p[2] = 0.0993 / 1.0993; p[3] = 1.0 / 4.5; p[4] = 0.08145;
      break;
    case 7:
      p[0] = 1.0 / 0.45; p[1] = 1.0 / 1.1115; p[2] = 0.1115 / 1.1115; p[3] = 0.25; p[4] = 0.0912;
      break;
    case 4: return cmsBuildGamma(nullptr, 2.2);
    case 5: return cmsBuildGamma(nullptr, 2.8);
    case 8: return cmsBuildGamma(nullptr, 1.0);
    case 13: break;
    default: flags |= RH_COLOR_ASSUMED; break;
  }
  return cmsBuildParametricToneCurve(nullptr, 4, p);
}

Profile profile_from_nclx(const heif_color_profile_nclx* nclx, uint32_t& flags) {
  if (!nclx) {
    flags |= RH_COLOR_ASSUMED;
    return Profile(cmsCreate_sRGBProfile());
  }
  const Primaries p = primaries_for(nclx->color_primaries, flags);
  std::unique_ptr<cmsToneCurve, decltype(&cmsFreeToneCurve)> curve(curve_for(nclx->transfer_characteristics, flags), cmsFreeToneCurve);
  if (!curve) throw std::bad_alloc();
  cmsToneCurve* curves[3] = {curve.get(), curve.get(), curve.get()};
  Profile result(cmsCreateRGBProfile(&p.white, &p.rgb, curves));
  if (!result.value) throw std::runtime_error("Cannot create a matching color profile.");
  return result;
}

std::vector<uint8_t> save_profile(cmsHPROFILE profile) {
  if (!profile) throw std::runtime_error("Cannot create an output color profile.");
  cmsUInt32Number length = 0;
  if (!cmsSaveProfileToMem(profile, nullptr, &length) || length == 0 || length > kMaxProfile)
    throw std::runtime_error("Cannot serialize the output color profile.");
  std::vector<uint8_t> result(length);
  if (!cmsSaveProfileToMem(profile, result.data(), &length))
    throw std::runtime_error("Cannot serialize the output color profile.");
  result.resize(length);
  // ICC's creation timestamp does not alter its color meaning. Normalize it
  // for reproducible profile bytes; Little CMS does not assign a profile ID.
  if (result.size() >= 36) {
    const uint8_t fixed_date[12] = {0x07, 0xe8, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0};
    std::memcpy(result.data() + 24, fixed_date, sizeof(fixed_date));
  }
  return result;
}

void copy_bytes(const std::vector<uint8_t>& data, uint8_t*& out, size_t& length) {
  if (data.empty()) return;
  out = static_cast<uint8_t*>(std::malloc(data.size()));
  if (!out) throw std::bad_alloc();
  std::memcpy(out, data.data(), data.size());
  length = data.size();
}

using Vec = std::array<double, 3>;
using Mat = std::array<std::array<double, 3>, 3>;
Vec multiply(const Mat& a, const Vec& b) {
  Vec r{};
  for (int i = 0; i < 3; ++i) for (int j = 0; j < 3; ++j) r[i] += a[i][j] * b[j];
  return r;
}
Mat multiply(const Mat& a, const Mat& b) {
  Mat r{};
  for (int i = 0; i < 3; ++i) for (int j = 0; j < 3; ++j)
    for (int k = 0; k < 3; ++k) r[i][j] += a[i][k] * b[k][j];
  return r;
}
Mat inverse(const Mat& a) {
  const double d = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
                 - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
                 + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
  if (!std::isfinite(d) || std::abs(d) < 1e-12) throw std::runtime_error("Invalid color primaries.");
  Mat r{};
  for (int i = 0; i < 3; ++i) for (int j = 0; j < 3; ++j) {
    const int row1 = (j + 1) % 3, row2 = (j + 2) % 3;
    const int col1 = (i + 1) % 3, col2 = (i + 2) % 3;
    r[i][j] = (a[row1][col1] * a[row2][col2] - a[row1][col2] * a[row2][col1]) / d;
  }
  return r;
}
Vec xy_xyz(cmsCIExyY v) { return {v.x / v.y, 1.0, (1.0 - v.x - v.y) / v.y}; }
Mat rgb_xyz(const Primaries& p) {
  const Vec r = xy_xyz(p.rgb.Red), g = xy_xyz(p.rgb.Green), b = xy_xyz(p.rgb.Blue);
  Mat m{{{r[0], g[0], b[0]}, {r[1], g[1], b[1]}, {r[2], g[2], b[2]}}};
  const Vec s = multiply(inverse(m), xy_xyz(p.white));
  for (int i = 0; i < 3; ++i) for (int j = 0; j < 3; ++j) m[i][j] *= s[j];
  return m;
}
Mat to_srgb_matrix(const Primaries& source) {
  uint32_t ignored = 0;
  const Primaries target = primaries_for(1, ignored);
  const Mat bradford{{{0.8951, 0.2664, -0.1614}, {-0.7502, 1.7135, 0.0367}, {0.0389, -0.0685, 1.0296}}};
  const Vec sw = multiply(bradford, xy_xyz(source.white));
  const Vec dw = multiply(bradford, xy_xyz(target.white));
  Mat diagonal{};
  for (int i = 0; i < 3; ++i) diagonal[i][i] = dw[i] / sw[i];
  const Mat adaptation = multiply(multiply(inverse(bradford), diagonal), bradford);
  return multiply(multiply(inverse(rgb_xyz(target)), adaptation), rgb_xyz(source));
}

struct Pixels {
  const uint8_t* data;
  size_t stride;
  uint32_t width;
  uint32_t height;
  int channels;
  int sample_bytes;
  double maximum;
  bool premultiplied;

  Vec read(uint32_t x, uint32_t y, double& alpha) const {
    const uint8_t* p = data + size_t(y) * stride + size_t(x) * channels * sample_bytes;
    auto sample = [&](int c) {
      const uint8_t* s = p + c * sample_bytes;
      const uint32_t value = sample_bytes == 1 ? s[0] : uint32_t(s[0]) | (uint32_t(s[1]) << 8);
      return unit(value / maximum);
    };
    alpha = channels == 4 ? sample(3) : 1.0;
    Vec rgb{sample(0), sample(1), sample(2)};
    if (premultiplied) {
      for (double& v : rgb) v = alpha > 0.0 ? unit(v / alpha) : 0.0;
    }
    return rgb;
  }
};

double pq_eotf(double encoded) {
  constexpr double m1 = 2610.0 / 16384.0, m2 = 2523.0 / 32.0;
  constexpr double c1 = 3424.0 / 4096.0, c2 = 2413.0 / 128.0, c3 = 2392.0 / 128.0;
  const double p = std::pow(unit(encoded), 1.0 / m2);
  return 10000.0 * std::pow(std::max(p - c1, 0.0) / std::max(c2 - c3 * p, 1e-12), 1.0 / m1);
}
double hlg_inverse(double encoded) {
  constexpr double a = 0.17883277, b = 0.28466892, c = 0.55991073;
  return encoded <= 0.5 ? encoded * encoded / 3.0 : (std::exp((encoded - c) / a) + b) / 12.0;
}

void hdr_to_sdr(const Pixels& input, uint8_t* out, const heif_color_profile_nclx* nclx,
                uint32_t& flags, bool preserve_alpha = false) {
  const Primaries primaries = primaries_for(nclx->color_primaries, flags, true);
  const Mat source_xyz = rgb_xyz(primaries);
  const Mat conversion = to_srgb_matrix(primaries);
  const bool pq = nclx->transfer_characteristics == heif_transfer_characteristic_ITU_R_BT_2100_0_PQ;
  auto to_nits = [&](Vec rgb) {
    if (pq) {
      for (double& v : rgb) v = pq_eotf(v);
    } else {
      for (double& v : rgb) v = hlg_inverse(v);
      const double scene_y = std::max(0.0, multiply(source_xyz, rgb)[1]);
      const double scale = 1000.0 * std::pow(scene_y, 0.2);
      for (double& v : rgb) v *= scale;
    }
    return multiply(conversion, rgb);
  };
  // Measure only visible pixels, so hidden RGB under alpha=0 cannot change exposure.
  double peak = kReferenceNits;
  for (uint32_t y = 0; y < input.height; ++y) for (uint32_t x = 0; x < input.width; ++x) {
    double alpha;
    const Vec rgb = to_nits(input.read(x, y, alpha));
    if (alpha > 0.0) peak = std::max(peak, 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]);
  }
  const double white = peak / kReferenceNits;
  for (uint32_t y = 0; y < input.height; ++y) for (uint32_t x = 0; x < input.width; ++x) {
    double alpha;
    Vec rgb = to_nits(input.read(x, y, alpha));
    const double luminance = std::max(0.0, 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]);
    const double normalized = luminance / kReferenceNits;
    const double mapped = normalized * (1.0 + normalized / (white * white)) / (1.0 + normalized);
    const double scale = luminance > 1e-12 ? mapped / luminance : 0.0;
    const size_t offset = (size_t(y) * input.width + x) * (preserve_alpha ? 4u : 3u);
    for (int c = 0; c < 3; ++c) {
      const double mapped_linear = unit(rgb[c] * scale);
      const double linear = preserve_alpha ? mapped_linear : mapped_linear * alpha + (1.0 - alpha);
      out[offset + c] = quantize(srgb_encode(linear));
    }
    if (preserve_alpha) out[offset + 3] = quantize(alpha);
  }
  flags |= RH_HDR_TONE_MAPPED;
}

void sdr_to_rgb8(const Pixels& input, uint8_t* out, cmsHPROFILE source,
                 bool source_gray, uint32_t& flags, bool preserve_alpha = false) {
  const bool alpha = input.channels == 4;
  Profile xyz, srgb;
  Transform to_xyz, from_xyz, gray_to_srgb;
  constexpr cmsUInt32Number transform_flags = cmsFLAGS_NOCACHE | cmsFLAGS_NOOPTIMIZE;
  if (source_gray) {
    srgb = Profile(cmsCreate_sRGBProfile());
    gray_to_srgb.value = cmsCreateTransform(source, TYPE_GRAY_FLT, srgb.value, TYPE_RGB_FLT,
                                            INTENT_RELATIVE_COLORIMETRIC, transform_flags);
    if (!gray_to_srgb.value) throw std::runtime_error("Cannot convert the grayscale color profile.");
    flags |= RH_GRAY_PROFILE_TO_SRGB;
  } else if (alpha && !preserve_alpha) {
    xyz = Profile(cmsCreateXYZProfile());
    to_xyz.value = cmsCreateTransform(source, TYPE_RGB_FLT, xyz.value, TYPE_XYZ_FLT,
                                      INTENT_RELATIVE_COLORIMETRIC, transform_flags);
    from_xyz.value = cmsCreateTransform(xyz.value, TYPE_XYZ_FLT, source, TYPE_RGB_FLT,
                                        INTENT_RELATIVE_COLORIMETRIC, transform_flags);
    if (!to_xyz.value || !from_xyz.value)
      throw std::runtime_error("Cannot composite alpha using this color profile.");
  }

  std::vector<float> row(size_t(input.width) * 3), work(alpha && !source_gray ? row.size() : 0);
  std::vector<float> gray(source_gray ? input.width : 0), alphas(alpha ? input.width : 0);
  const cmsCIEXYZ* white = cmsD50_XYZ();
  for (uint32_t y = 0; y < input.height; ++y) {
    bool needs_alpha = false;
    for (uint32_t x = 0; x < input.width; ++x) {
      double a;
      const Vec rgb = input.read(x, y, a);
      for (int c = 0; c < 3; ++c) row[size_t(x) * 3 + c] = static_cast<float>(rgb[c]);
      if (source_gray) gray[x] = static_cast<float>(rgb[0]);
      if (alpha) {
        alphas[x] = static_cast<float>(a);
        needs_alpha = needs_alpha || (!preserve_alpha && a < 1.0);
      }
    }
    if (source_gray) cmsDoTransform(gray_to_srgb.value, gray.data(), row.data(), input.width);
    if (needs_alpha && source_gray) {
      for (uint32_t x = 0; x < input.width; ++x) for (int c = 0; c < 3; ++c) {
        const size_t i = size_t(x) * 3 + c;
        row[i] = static_cast<float>(srgb_encode(srgb_linear(row[i]) * alphas[x] + 1.0 - alphas[x]));
      }
    } else if (needs_alpha) {
      cmsDoTransform(to_xyz.value, row.data(), work.data(), input.width);
      const double w[3] = {white->X, white->Y, white->Z};
      for (uint32_t x = 0; x < input.width; ++x) for (int c = 0; c < 3; ++c) {
        const size_t i = size_t(x) * 3 + c;
        work[i] = static_cast<float>(work[i] * alphas[x] + w[c] * (1.0 - alphas[x]));
      }
      cmsDoTransform(from_xyz.value, work.data(), work.data(), input.width);
      for (uint32_t x = 0; x < input.width; ++x) if (alphas[x] < 1.0f) {
        for (int c = 0; c < 3; ++c) row[size_t(x) * 3 + c] = work[size_t(x) * 3 + c];
      }
    }
    const size_t offset = size_t(y) * input.width * (preserve_alpha ? 4u : 3u);
    if (preserve_alpha) {
      for (uint32_t x = 0; x < input.width; ++x) {
        for (int c = 0; c < 3; ++c) out[offset + size_t(x) * 4 + c] = quantize(row[size_t(x) * 3 + c]);
        out[offset + size_t(x) * 4 + 3] = alpha ? quantize(alphas[x]) : 255;
      }
    } else {
      for (size_t i = 0; i < row.size(); ++i) out[offset + i] = quantize(row[i]);
    }
  }
}

void read_metadata(Document& doc, heif_image_handle* handle, heif_item_id id, RhImage& out) {
  if (heif_item_get_properties_of_type(doc.context, id, heif_item_property_type_transform_rotation, nullptr, 0) > 0 ||
      heif_item_get_properties_of_type(doc.context, id, heif_item_property_type_transform_mirror, nullptr, 0) > 0)
    out.flags |= RH_HEIF_ORIENTATION;
  heif_item_id metadata_id = 0;
  if (heif_image_handle_get_list_of_metadata_block_IDs(handle, "Exif", &metadata_id, 1) == 1) {
    const size_t n = heif_image_handle_get_metadata_size(handle, metadata_id);
    if (n > kMaxExif) { out.flags |= RH_EXIF_OMITTED; return; }
    if (n) {
      std::vector<uint8_t> bytes(n);
      const heif_error status = heif_image_handle_get_metadata(handle, metadata_id, bytes.data());
      if (status.code == heif_error_Ok) copy_bytes(bytes, out.exif, out.exif_len);
      else out.flags |= RH_EXIF_OMITTED | RH_DECODER_WARNING;
    }
  }
}

size_t validate_pixels(const uint8_t* data, size_t length, uint32_t width,
                        uint32_t height, uint32_t channels) {
  if (!data || !width || !height || width > kMaxDimension || height > kMaxDimension ||
      uint64_t(width) * height > kMaxPixels)
    throw std::runtime_error("Invalid pixels or image dimensions exceed the 200 megapixel safety limit.");
  if (channels != 3 && channels != 4)
    throw std::runtime_error("Pixels must be packed RGB8 or straight RGBA8.");
  const uint64_t expected = uint64_t(width) * height * channels;
  if (expected > std::numeric_limits<size_t>::max() || length != static_cast<size_t>(expected))
    throw std::runtime_error("The pixel buffer length does not match the image dimensions.");
  return static_cast<size_t>(expected);
}

Profile validated_rgb_profile(const uint8_t* icc, size_t length, uint32_t& flags) {
  if (!length) {
    flags |= RH_COLOR_ASSUMED;
    Profile srgb(cmsCreate_sRGBProfile());
    if (!srgb.value) throw std::bad_alloc();
    return srgb;
  }
  if (!icc || length > kMaxProfile)
    throw std::runtime_error("The RGB ICC profile is missing or exceeds the 4 MiB safety limit.");
  Profile result(cmsOpenProfileFromMem(icc, static_cast<cmsUInt32Number>(length)));
  if (!result.value || cmsGetColorSpace(result.value) != cmsSigRgbData)
    throw std::runtime_error("The supplied ICC profile is invalid or is not an RGB profile.");
  return result;
}

heif_error write_encoded_bytes(heif_context*, const void* data, size_t size, void* opaque) noexcept {
  try {
    auto& buffer = *static_cast<std::vector<uint8_t>*>(opaque);
    if (size > kMaxInput - buffer.size())
      return {heif_error_Memory_allocation_error, heif_suberror_Security_limit_exceeded,
              "The encoded HEIC exceeds the 2 GiB safety limit."};
    if (size) {
      const auto* bytes = static_cast<const uint8_t*>(data);
      buffer.insert(buffer.end(), bytes, bytes + size);
    }
    return {heif_error_Ok, heif_suberror_Unspecified, nullptr};
  } catch (...) {
    return {heif_error_Memory_allocation_error, heif_suberror_Unspecified,
            "Not enough memory to assemble the HEIC file."};
  }
}
} // namespace

extern "C" void* rh_open(const uint8_t* data, size_t len, char* err, size_t errlen) {
  error_text(err, errlen, "");
  try {
    if (!data || len < 12) throw std::runtime_error("The file is empty or too short to be HEIC.");
    if (len > kMaxInput) throw std::runtime_error("The input file exceeds the 2 GiB safety limit.");
    auto document = std::make_unique<Document>();
    check(heif_init(nullptr));
    document->initialized = true;
    document->context = heif_context_alloc();
    if (!document->context) throw std::bad_alloc();
    heif_security_limits* limits = heif_context_get_security_limits(document->context);
    if (limits) {
      limits->max_image_size_pixels = kMaxPixels;
      limits->max_color_profile_size = static_cast<uint32_t>(kMaxProfile);
      limits->max_items = 16384;
      limits->max_total_memory = uint64_t{3} * 1024u * 1024u * 1024u;
      limits->max_memory_block_size = uint64_t{2} * 1024u * 1024u * 1024u;
    }
    heif_context_set_max_decoding_threads(document->context, 2);
    check(heif_context_read_from_memory_without_copy(document->context, data, len, nullptr));
    const int count = heif_context_get_number_of_top_level_images(document->context);
    if (count <= 0) throw std::runtime_error("The file contains no supported still image.");
    if (count > 4096) throw std::runtime_error("The file contains too many top-level images.");
    document->ids.resize(static_cast<size_t>(count));
    const int actual = heif_context_get_list_of_top_level_image_IDs(document->context, document->ids.data(), count);
    if (actual <= 0 || actual > count) throw std::runtime_error("Cannot enumerate images in the HEIC file.");
    document->ids.resize(static_cast<size_t>(actual));
    heif_item_id primary;
    check(heif_context_get_primary_image_ID(document->context, &primary));
    const auto first = std::find(document->ids.begin(), document->ids.end(), primary);
    if (first != document->ids.end()) std::rotate(document->ids.begin(), first, first + 1);
    return document.release();
  } catch (const std::bad_alloc&) {
    error_text(err, errlen, "Not enough memory to open this image.");
  } catch (const std::exception& e) {
    error_text(err, errlen, e.what());
  } catch (...) {
    error_text(err, errlen, "Unexpected error while opening the HEIC file.");
  }
  return nullptr;
}

extern "C" int rh_count(void* document) {
  if (!document) return 0;
  return static_cast<int>(static_cast<Document*>(document)->ids.size());
}

static int decode_image(void* opaque, int index, RhImage* out, char* err, size_t errlen,
                        bool preserve_alpha) {
  error_text(err, errlen, "");
  if (out) std::memset(out, 0, sizeof(*out));
  try {
    if (!opaque || !out) throw std::runtime_error("Invalid decoder state.");
    Document& document = *static_cast<Document*>(opaque);
    if (index < 0 || size_t(index) >= document.ids.size()) throw std::runtime_error("Invalid image index.");
    heif_image_handle* handle_raw = nullptr;
    const heif_error handle_status = heif_context_get_image_handle(document.context, document.ids[size_t(index)], &handle_raw);
    Handle handle(handle_raw, heif_image_handle_release);
    check(handle_status);
    ImageGuard guard;
    RhImage& result = guard.image;
    const int signalled_width = heif_image_handle_get_width(handle.get());
    const int signalled_height = heif_image_handle_get_height(handle.get());
    if (signalled_width <= 0 || signalled_height <= 0 || signalled_width > int(kMaxDimension) ||
        signalled_height > int(kMaxDimension) || uint64_t(signalled_width) * uint64_t(signalled_height) > kMaxPixels)
      throw std::runtime_error("Image dimensions exceed 65,535 pixels per side or the 200 megapixel safety limit.");

    heif_color_profile_nclx* nclx_raw = nullptr;
    const heif_error nclx_status = heif_image_handle_get_nclx_color_profile(handle.get(), &nclx_raw);
    Nclx nclx(nclx_raw, heif_nclx_color_profile_free);
    if (nclx_status.code != heif_error_Ok) nclx.reset();
    const bool source_has_nclx = bool(nclx);
    Options options(heif_decoding_options_alloc(), heif_decoding_options_free);
    if (!options) throw std::bad_alloc();
    options->ignore_transformations = 0;
    options->convert_hdr_to_8bit = 0;
    options->output_image_nclx_profile_passthrough = 1;
    options->color_conversion_options.preferred_chroma_upsampling_algorithm = heif_chroma_upsampling_bilinear;
    options->color_conversion_options.only_use_preferred_chroma_algorithm = 1;
    options->num_codec_threads = 2;
    const int source_bits = heif_image_handle_get_luma_bits_per_pixel(handle.get());
    if (source_bits > 16) throw std::runtime_error("Unsupported HEIC component precision.");
    const bool high = source_bits > 8;
    const bool alpha = heif_image_handle_has_alpha_channel(handle.get()) != 0;
    const heif_chroma chroma = high
      ? (alpha ? heif_chroma_interleaved_RRGGBBAA_LE : heif_chroma_interleaved_RRGGBB_LE)
      : (alpha ? heif_chroma_interleaved_RGBA : heif_chroma_interleaved_RGB);
    heif_image* decoded_raw = nullptr;
    const heif_error decode_status = heif_decode_image(handle.get(), &decoded_raw, heif_colorspace_RGB, chroma, options.get());
    Decoded decoded(decoded_raw, heif_image_release);
    check(decode_status);
    if (!decoded) throw std::runtime_error("The decoder returned no image.");
    const int width = heif_image_get_width(decoded.get(), heif_channel_interleaved);
    const int height = heif_image_get_height(decoded.get(), heif_channel_interleaved);
    if (width <= 0 || height <= 0 || width > int(kMaxDimension) || height > int(kMaxDimension) ||
        uint64_t(width) * uint64_t(height) > kMaxPixels)
      throw std::runtime_error("Decoded dimensions exceed 65,535 pixels per side or the 200 megapixel safety limit.");
    result.width = static_cast<uint32_t>(width);
    result.height = static_cast<uint32_t>(height);
    const int channels = alpha ? 4 : 3;
    const int bits = heif_image_get_bits_per_pixel_range(decoded.get(), heif_channel_interleaved);
    const int storage_bits = heif_image_get_bits_per_pixel(decoded.get(), heif_channel_interleaved);
    const int sample_bytes = storage_bits / (channels * 8);
    if ((sample_bytes != 1 && sample_bytes != 2) || storage_bits != sample_bytes * channels * 8 ||
        bits <= 0 || bits > 16 || bits > sample_bytes * 8)
      throw std::runtime_error("The decoder returned an unsupported pixel layout.");
    size_t stride = 0;
    const uint8_t* plane = heif_image_get_plane_readonly2(decoded.get(), heif_channel_interleaved, &stride);
    const size_t row_bytes = size_t(width) * channels * sample_bytes;
    if (!plane || stride < row_bytes || stride > std::numeric_limits<size_t>::max() / size_t(height))
      throw std::runtime_error("The decoder returned an invalid pixel stride.");
    const Pixels pixels{plane, stride, result.width, result.height, channels, sample_bytes,
                        double((uint32_t{1} << bits) - 1), heif_image_is_premultiplied_alpha(decoded.get()) != 0};
    if (bits > 8 || source_bits > 8) result.flags |= RH_HIGH_DEPTH_TO_8;
    if (alpha) result.flags |= preserve_alpha ? RH_ALPHA_PRESERVED : RH_ALPHA_ON_WHITE;
    if (heif_image_get_decoding_warnings(decoded.get(), 0, nullptr, 0) > 0) result.flags |= RH_DECODER_WARNING;
    if (!nclx) {
      nclx_raw = nullptr;
      const heif_error e = heif_image_get_nclx_color_profile(decoded.get(), &nclx_raw);
      if (e.code == heif_error_Ok) nclx.reset(nclx_raw);
      else if (nclx_raw) heif_nclx_color_profile_free(nclx_raw);
    }
    const bool hdr = nclx && (nclx->transfer_characteristics == heif_transfer_characteristic_ITU_R_BT_2100_0_PQ ||
                              nclx->transfer_characteristics == heif_transfer_characteristic_ITU_R_BT_2100_0_HLG);

    result.pixels_len = size_t(width) * size_t(height) * (preserve_alpha ? 4u : 3u);
    result.pixels = static_cast<uint8_t*>(std::malloc(result.pixels_len));
    if (!result.pixels) throw std::bad_alloc();
    std::vector<uint8_t> profile_bytes;
    if (hdr) {
      hdr_to_sdr(pixels, result.pixels, nclx.get(), result.flags, preserve_alpha);
      Profile srgb(cmsCreate_sRGBProfile());
      profile_bytes = save_profile(srgb.value);
    } else {
      Profile source;
      bool gray = false;
      const size_t profile_size = heif_image_handle_get_raw_color_profile_size(handle.get());
      if (profile_size > 0 && profile_size <= kMaxProfile) {
        profile_bytes.resize(profile_size);
        const heif_error e = heif_image_handle_get_raw_color_profile(handle.get(), profile_bytes.data());
        if (e.code == heif_error_Ok) {
          source = Profile(cmsOpenProfileFromMem(profile_bytes.data(), static_cast<cmsUInt32Number>(profile_size)));
          if (source.value) {
            const cmsColorSpaceSignature space = cmsGetColorSpace(source.value);
            gray = space == cmsSigGrayData;
            if (space != cmsSigRgbData && !gray) source = Profile();
          }
        }
        if (!source.value) { profile_bytes.clear(); result.flags |= RH_ICC_OMITTED | RH_COLOR_ASSUMED; }
      } else if (profile_size > kMaxProfile) {
        result.flags |= RH_ICC_OMITTED | RH_COLOR_ASSUMED;
      }
      if (!source.value) {
        if (!source_has_nclx) result.flags |= RH_COLOR_ASSUMED;
        source = profile_from_nclx(nclx.get(), result.flags);
        profile_bytes = save_profile(source.value);
      }
      sdr_to_rgb8(pixels, result.pixels, source.value, gray, result.flags, preserve_alpha);
      if (gray) {
        Profile srgb(cmsCreate_sRGBProfile());
        profile_bytes = save_profile(srgb.value);
      }
    }
    copy_bytes(profile_bytes, result.icc, result.icc_len);
    read_metadata(document, handle.get(), document.ids[size_t(index)], result);
    *out = result;
    result = RhImage{};
    return 1;
  } catch (const std::bad_alloc&) {
    error_text(err, errlen, "Not enough memory to decode this image at its original size.");
  } catch (const std::exception& e) {
    error_text(err, errlen, e.what());
  } catch (...) {
    error_text(err, errlen, "Unexpected error while converting this image.");
  }
  return 0;
}

extern "C" int rh_decode(void* document, int index, RhImage* out, char* err, size_t errlen) {
  return decode_image(document, index, out, err, errlen, false);
}

extern "C" int rh_decode_rgba(void* document, int index, RhImage* out, char* err, size_t errlen) {
  return decode_image(document, index, out, err, errlen, true);
}

extern "C" void rh_free_image(RhImage* image) {
  if (!image) return;
  std::free(image->pixels);
  std::free(image->icc);
  std::free(image->exif);
  std::memset(image, 0, sizeof(*image));
}

extern "C" void rh_close(void* document) { delete static_cast<Document*>(document); }
extern "C" const char* rh_version(void) { return heif_get_version(); }

extern "C" void rh_free_buffer(RhBuffer* buffer) {
  if (!buffer) return;
  std::free(buffer->data);
  std::memset(buffer, 0, sizeof(*buffer));
}

extern "C" int rh_srgb_profile(RhBuffer* out, char* err, size_t errlen) {
  error_text(err, errlen, "");
  if (out) std::memset(out, 0, sizeof(*out));
  try {
    if (!out) throw std::runtime_error("Invalid output buffer.");
    Profile srgb(cmsCreate_sRGBProfile());
    const std::vector<uint8_t> bytes = save_profile(srgb.value);
    copy_bytes(bytes, out->data, out->len);
    return 1;
  } catch (const std::bad_alloc&) {
    error_text(err, errlen, "Not enough memory to create the sRGB profile.");
  } catch (const std::exception& e) {
    error_text(err, errlen, e.what());
  } catch (...) {
    error_text(err, errlen, "Unexpected error while creating the sRGB profile.");
  }
  return 0;
}

extern "C" int rh_nclx_profile(uint32_t primaries, uint32_t transfer,
                                uint32_t matrix, uint32_t full_range,
                                RhBuffer* out, char* err, size_t errlen) {
  error_text(err, errlen, "");
  if (out) std::memset(out, 0, sizeof(*out));
  try {
    if (!out) throw std::runtime_error("Invalid output buffer.");
    if (primaries > 255 || transfer > 255)
      throw std::runtime_error("Unsupported cICP color code.");
    if (matrix != 0)
      throw std::runtime_error("PNG cICP must use RGB identity matrix coefficients (0).");
    if (full_range != 1)
      throw std::runtime_error("Narrow-range PNG transcoding is not supported; retain the original PNG.");
    if (transfer == 16 || transfer == 18)
      throw std::runtime_error("HDR PNG transcoding is not supported; retain the original PNG.");
    heif_color_profile_nclx nclx{};
    nclx.color_primaries = static_cast<heif_color_primaries>(primaries);
    nclx.transfer_characteristics = static_cast<heif_transfer_characteristics>(transfer);
    nclx.matrix_coefficients = heif_matrix_coefficients_RGB_GBR;
    nclx.full_range_flag = 1;
    uint32_t flags = 0;
    Profile profile = profile_from_nclx(&nclx, flags);
    if (flags & RH_COLOR_ASSUMED)
      throw std::runtime_error("The PNG cICP primaries or transfer function are unsupported; retain the original PNG.");
    const std::vector<uint8_t> bytes = save_profile(profile.value);
    copy_bytes(bytes, out->data, out->len);
    return 1;
  } catch (const std::bad_alloc&) {
    error_text(err, errlen, "Not enough memory to create the PNG color profile.");
  } catch (const std::exception& e) {
    error_text(err, errlen, e.what());
  } catch (...) {
    error_text(err, errlen, "Unexpected error while creating the PNG color profile.");
  }
  return 0;
}

extern "C" int rh_flatten_rgba(const uint8_t* rgba, size_t rgba_len,
                               uint32_t width, uint32_t height,
                               const uint8_t* icc, size_t icc_len,
                               RhBuffer* out, char* err, size_t errlen) {
  error_text(err, errlen, "");
  if (out) std::memset(out, 0, sizeof(*out));
  try {
    if (!out) throw std::runtime_error("Invalid output buffer.");
    validate_pixels(rgba, rgba_len, width, height, 4);
    BufferGuard guard;
    RhBuffer& result = guard.buffer;
    Profile source = validated_rgb_profile(icc, icc_len, result.flags);
    result.len = size_t(width) * height * 3;
    result.data = static_cast<uint8_t*>(std::malloc(result.len));
    if (!result.data) throw std::bad_alloc();
    const Pixels pixels{rgba, size_t(width) * 4, width, height, 4, 1, 255.0, false};
    sdr_to_rgb8(pixels, result.data, source.value, false, result.flags);
    // An opaque RGBA image needs no transparency warning.
    for (size_t i = 3; i < rgba_len; i += 4) {
      if (rgba[i] != 255) { result.flags |= RH_ALPHA_ON_WHITE; break; }
    }
    *out = result;
    result = RhBuffer{};
    return 1;
  } catch (const std::bad_alloc&) {
    error_text(err, errlen, "Not enough memory to composite transparency.");
  } catch (const std::exception& e) {
    error_text(err, errlen, e.what());
  } catch (...) {
    error_text(err, errlen, "Unexpected error while compositing transparency.");
  }
  return 0;
}

extern "C" int rh_encode_heic(const uint8_t* pixels, size_t pixels_len,
                              uint32_t width, uint32_t height, uint32_t channels,
                              const uint8_t* icc, size_t icc_len,
                              const uint8_t* exif_tiff, size_t exif_len,
                              RhBuffer* out, char* err, size_t errlen) {
  error_text(err, errlen, "");
  if (out) std::memset(out, 0, sizeof(*out));
  try {
    if (!out) throw std::runtime_error("Invalid output buffer.");
    validate_pixels(pixels, pixels_len, width, height, channels);
    // x265 pads each coded dimension to an even value of at least 64 pixels.
    // HEIF clean-aperture metadata retains the exact, potentially odd size.
    const uint64_t coded_width = std::max<uint64_t>(64, (uint64_t(width) + 1) & ~uint64_t{1});
    const uint64_t coded_height = std::max<uint64_t>(64, (uint64_t(height) + 1) & ~uint64_t{1});
    if (coded_width * coded_height > 142606336)
      throw std::runtime_error("HEIC encoding exceeds the HEVC limit of 142,606,336 coded pixels.");
    if (exif_len > kMaxExif || (exif_len && !exif_tiff))
      throw std::runtime_error("The EXIF metadata is missing or exceeds the 16 MiB safety limit.");
    if (exif_len && (exif_len < 8 ||
        !(std::memcmp(exif_tiff, "II\x2a\0", 4) == 0 || std::memcmp(exif_tiff, "MM\0\x2a", 4) == 0)))
      throw std::runtime_error("EXIF must be normalized TIFF data without a container prefix.");

    BufferGuard guard;
    RhBuffer& result = guard.buffer;
    Profile profile = validated_rgb_profile(icc, icc_len, result.flags);
    std::vector<uint8_t> generated_profile;
    if (!icc_len) {
      generated_profile = save_profile(profile.value);
      icc = generated_profile.data();
      icc_len = generated_profile.size();
    }

    Document document;
    check(heif_init(nullptr));
    document.initialized = true;
    document.context = heif_context_alloc();
    if (!document.context) throw std::bad_alloc();
    heif_security_limits* limits = heif_context_get_security_limits(document.context);
    if (limits) {
      limits->max_image_size_pixels = kMaxPixels;
      limits->max_color_profile_size = static_cast<uint32_t>(kMaxProfile);
      limits->max_total_memory = uint64_t{3} * 1024u * 1024u * 1024u;
      limits->max_memory_block_size = uint64_t{2} * 1024u * 1024u * 1024u;
    }
    const heif_encoder_descriptor* descriptor = nullptr;
    if (heif_get_encoder_descriptors(heif_compression_HEVC, "x265", &descriptor, 1) != 1 ||
        !descriptor || !heif_encoder_descriptor_supports_lossless_compression(descriptor))
      throw std::runtime_error("The built-in lossless HEIC encoder is unavailable.");
    heif_encoder* raw_encoder = nullptr;
    const heif_error encoder_status = heif_context_get_encoder(document.context, descriptor, &raw_encoder);
    Encoder encoder(raw_encoder, heif_encoder_release);
    check(encoder_status);
    check(heif_encoder_set_lossy_quality(encoder.get(), 100));
    check(heif_encoder_set_lossless(encoder.get(), 1));
    check(heif_encoder_set_logging_level(encoder.get(), 0));
    check(heif_encoder_set_parameter(encoder.get(), "chroma", "444"));
    check(heif_encoder_set_parameter(encoder.get(), "preset", "fast"));
    check(heif_encoder_set_parameter(encoder.get(), "x265:pools", "2"));
    check(heif_encoder_set_parameter(encoder.get(), "x265:frame-threads", "1"));
    // A fixed CTU size avoids x265's process-global CTU configuration varying
    // between simultaneously open jobs. Its own padding handles tiny images.
    check(heif_encoder_set_parameter(encoder.get(), "x265:ctu", "64"));

    heif_image* raw_image = nullptr;
    const heif_error image_status = heif_image_create(static_cast<int>(width), static_cast<int>(height),
        heif_colorspace_RGB, channels == 4 ? heif_chroma_interleaved_RGBA : heif_chroma_interleaved_RGB,
        &raw_image);
    Decoded image(raw_image, heif_image_release);
    check(image_status);
    check(heif_image_add_plane_safe(image.get(), heif_channel_interleaved,
          static_cast<int>(width), static_cast<int>(height), 8, limits));
    size_t stride = 0;
    uint8_t* plane = heif_image_get_plane2(image.get(), heif_channel_interleaved, &stride);
    const size_t row_size = size_t(width) * channels;
    if (!plane || stride < row_size || stride > std::numeric_limits<size_t>::max() / height)
      throw std::runtime_error("The HEIC encoder allocated an invalid pixel plane.");
    for (uint32_t y = 0; y < height; ++y)
      std::memcpy(plane + size_t(y) * stride, pixels + size_t(y) * row_size, row_size);
    heif_image_set_premultiplied_alpha(image.get(), 0);
    check(heif_image_set_raw_color_profile(image.get(), "prof", icc, icc_len));

    Nclx nclx(heif_nclx_color_profile_alloc(), heif_nclx_color_profile_free);
    if (!nclx) throw std::bad_alloc();
    // ICC is authoritative for arbitrary native RGB profiles. Unspecified
    // NCLX primaries/transfer avoid falsely labelling a P3 or custom ICC as sRGB.
    // Matrix zero is essential: it encodes a channel permutation, not a lossy
    // RGB-to-YCbCr conversion. libheif's own lossless RGB example does this.
    const bool assumed_srgb = !generated_profile.empty();
    check(heif_nclx_color_profile_set_color_primaries(nclx.get(), assumed_srgb ? 1 : 2));
    check(heif_nclx_color_profile_set_transfer_characteristics(nclx.get(), assumed_srgb ? 13 : 2));
    check(heif_nclx_color_profile_set_matrix_coefficients(nclx.get(), 0));
    nclx->full_range_flag = 1;
    check(heif_image_set_nclx_color_profile(image.get(), nclx.get()));
    EncodeOptions options(heif_encoding_options_alloc(), heif_encoding_options_free);
    if (!options) throw std::bad_alloc();
    options->save_alpha_channel = 1;
    options->save_two_colr_boxes_when_ICC_and_nclx_available = 1;
    options->output_nclx_profile = nclx.get();
    options->image_orientation = heif_orientation_normal;
    heif_image_handle* raw_handle = nullptr;
    const heif_error encoded_status = heif_context_encode_image(document.context, image.get(), encoder.get(),
                                                               options.get(), &raw_handle);
    Handle handle(raw_handle, heif_image_handle_release);
    check(encoded_status);
    check(heif_context_set_primary_image(document.context, handle.get()));
    if (exif_len)
      check(heif_context_add_exif_metadata(document.context, handle.get(), exif_tiff, static_cast<int>(exif_len)));
    std::vector<uint8_t> bytes;
    heif_writer writer{1, write_encoded_bytes};
    check(heif_context_write(document.context, &writer, &bytes));
    if (bytes.empty()) throw std::runtime_error("The HEIC encoder returned an empty file.");
    copy_bytes(bytes, result.data, result.len);
    result.flags |= RH_HEIC_LOSSLESS;
    if (channels == 4) result.flags |= RH_ALPHA_PRESERVED;
    *out = result;
    result = RhBuffer{};
    return 1;
  } catch (const std::bad_alloc&) {
    error_text(err, errlen, "Not enough memory to encode HEIC at its original size.");
  } catch (const std::exception& e) {
    error_text(err, errlen, e.what());
  } catch (...) {
    error_text(err, errlen, "Unexpected error while encoding the HEIC image.");
  }
  return 0;
}
