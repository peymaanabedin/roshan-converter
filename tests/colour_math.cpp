// Targeted checks for color math whose correctness affects pixel appearance.
#include "../native/bridge.cpp"
#include <cassert>
#include <cstdio>

int main() {
  assert(std::abs(pq_eotf(0.0)) < 1e-8);
  assert(std::abs(pq_eotf(1.0) - 10000.0) < 1e-6);
  assert(std::abs(pq_eotf(0.5080784215) - 100.0) < 0.0001);
  assert(std::abs(hlg_inverse(0.5) - 1.0 / 12.0) < 1e-9);
  assert(std::abs(hlg_inverse(1.0) - 1.0) < 1e-7);

  uint32_t flags = 0;
  Mat identity = to_srgb_matrix(primaries_for(1, flags));
  for (int i = 0; i < 3; ++i) for (int j = 0; j < 3; ++j)
    assert(std::abs(identity[i][j] - (i == j ? 1.0 : 0.0)) < 1e-10);
  Mat bt2020 = to_srgb_matrix(primaries_for(9, flags));
  assert(std::abs(bt2020[0][0] - 1.660491) < 0.000002);
  assert(std::abs(bt2020[0][1] + 0.587641) < 0.000002);
  assert(std::abs(bt2020[2][2] - 1.118730) < 0.000002);
  assert(flags == 0);

  cmsToneCurve* curve = curve_for(13, flags);
  assert(curve);
  assert(std::abs(cmsEvalToneCurveFloat(curve, 0.5f) - 0.21404114) < 0.000002);
  cmsFreeToneCurve(curve);

  char error[512]{};
  RhBuffer cicp{};
  assert(rh_nclx_profile(12, 13, 0, 1, &cicp, error, sizeof(error)) == 1);
  assert(cicp.len >= 128 && cicp.flags == 0);
  Profile p3_icc(cmsOpenProfileFromMem(cicp.data, static_cast<cmsUInt32Number>(cicp.len)));
  auto* red_curve = static_cast<cmsToneCurve*>(cmsReadTag(p3_icc.value, cmsSigRedTRCTag));
  assert(red_curve && std::abs(cmsEvalToneCurveFloat(red_curve, 0.5f) - 0.21404114) < 0.0001);
  const auto* red_xyz = static_cast<const cmsCIEXYZ*>(cmsReadTag(p3_icc.value, cmsSigRedColorantTag));
  assert(red_xyz && red_xyz->X > 0.51 && red_xyz->X < 0.53);
  rh_free_buffer(&cicp);
  for (const auto& bad : {std::array<uint32_t, 4>{2, 2, 0, 1}, {12, 13, 1, 1}, {1, 13, 0, 0}, {9, 16, 0, 1}, {9, 18, 0, 1}}) {
    assert(rh_nclx_profile(bad[0], bad[1], bad[2], bad[3], &cicp, error, sizeof(error)) == 0);
    assert(cicp.data == nullptr && cicp.len == 0);
  }

  const uint8_t premultiplied[4] = {128, 0, 0, 128};
  const Pixels alpha{premultiplied, 4, 1, 1, 4, 1, 255.0, true};
  double alpha_value;
  Vec rgb = alpha.read(0, 0, alpha_value);
  assert(rgb[0] == 1.0 && rgb[1] == 0.0 && std::abs(alpha_value - 128.0 / 255.0) < 1e-12);
  Profile profile(cmsCreate_sRGBProfile());
  uint8_t output[3]{};
  sdr_to_rgb8(alpha, output, profile.value, false, flags);
  assert(output[0] == 255 && output[1] >= 187 && output[1] <= 188 && output[2] >= 187 && output[2] <= 188);
  uint8_t straight_rgba[4]{};
  sdr_to_rgb8(alpha, straight_rgba, profile.value, false, flags, true);
  assert(straight_rgba[0] == 255 && straight_rgba[1] == 0 && straight_rgba[2] == 0 && straight_rgba[3] == 128);

  const uint8_t hi_bytes[6] = {0, 2, 255, 3, 255, 1}; // 10-bit: 512,1023,511
  const Pixels high{hi_bytes, 6, 1, 1, 3, 2, 1023.0, false};
  sdr_to_rgb8(high, output, profile.value, false, flags);
  assert(output[0] == 128 && output[1] == 255 && output[2] == 127);

  // PQ-encoded neutral 0, 100, 203, and 1000 nit values at 16-bit precision.
  std::vector<uint8_t> ramp(4 * 3 * 2);
  const double encoded[] = {0.0, 0.5080784215, 0.5806888810, 0.7518270962};
  for (int x = 0; x < 4; ++x) for (int c = 0; c < 3; ++c) {
    const uint16_t v = static_cast<uint16_t>(std::lround(encoded[x] * 65535.0));
    ramp[size_t(x) * 6 + c * 2] = static_cast<uint8_t>(v & 255);
    ramp[size_t(x) * 6 + c * 2 + 1] = static_cast<uint8_t>(v >> 8);
  }
  const Pixels pq{ramp.data(), 24, 4, 1, 3, 2, 65535.0, false};
  heif_color_profile_nclx nclx{};
  nclx.color_primaries = heif_color_primaries_ITU_R_BT_2020_2_and_2100_0;
  nclx.transfer_characteristics = heif_transfer_characteristic_ITU_R_BT_2100_0_PQ;
  uint8_t mapped[12]{};
  hdr_to_sdr(pq, mapped, &nclx, flags);
  assert((flags & RH_HDR_TONE_MAPPED) != 0);
  assert(mapped[0] == 0 && mapped[9] == 255);
  assert(mapped[3] >= 156 && mapped[3] <= 158);
  assert(mapped[6] >= 190 && mapped[6] <= 192);
  for (int x = 0; x < 4; ++x) assert(mapped[x * 3] == mapped[x * 3 + 1] && mapped[x * 3] == mapped[x * 3 + 2]);
  uint8_t mapped_rgba[16]{};
  hdr_to_sdr(pq, mapped_rgba, &nclx, flags, true);
  for (int x = 0; x < 4; ++x) {
    for (int c = 0; c < 3; ++c) assert(mapped_rgba[x * 4 + c] == mapped[x * 3 + c]);
    assert(mapped_rgba[x * 4 + 3] == 255);
  }
  std::puts("PASS: PQ, HLG, gamut matrices, ICC transfer curve, strict SDR cICP profiles, premultiplied linear alpha and straight RGBA, 10-bit range, and neutral HDR RGB/RGBA tone mapping.");
}
