#include "bridge.h"
#include <cstdio>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

int main(int argc, char** argv) {
  int failures = 0;
  std::printf("libheif=%s\n", rh_version());
  for (int a = 1; a < argc; ++a) {
    const std::string path = argv[a];
    std::ifstream stream(path, std::ios::binary);
    std::vector<uint8_t> input{std::istreambuf_iterator<char>(stream), {}};
    char error[1024]{};
    void* document = rh_open(input.data(), input.size(), error, sizeof(error));
    if (!document) { std::printf("%s OPEN_ERROR %s\n", path.c_str(), error); ++failures; continue; }
    const int count = rh_count(document);
    for (int i = 0; i < count; ++i) {
      RhImage image{};
      if (!rh_decode(document, i, &image, error, sizeof(error))) {
        std::printf("%s [%d/%d] DECODE_ERROR %s\n", path.c_str(), i + 1, count, error);
        ++failures;
      } else {
        const size_t center = (size_t(image.height / 2) * image.width + image.width / 2) * 3;
        std::printf("%s [%d/%d] %ux%u flags=%u bytes=%zu icc=%zu exif=%zu corner=%u,%u,%u center=%u,%u,%u\n",
          path.c_str(), i + 1, count, image.width, image.height, image.flags,
          image.pixels_len, image.icc_len, image.exif_len,
          image.pixels[0], image.pixels[1], image.pixels[2], image.pixels[center], image.pixels[center + 1], image.pixels[center + 2]);
      }
      rh_free_image(&image);
    }
    rh_close(document);
  }
  return failures ? 1 : 0;
}
