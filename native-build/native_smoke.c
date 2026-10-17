#include <stdio.h>
#include <stdint.h>
#include <libheif/heif.h>
#include <libde265/de265.h>
#include <lcms2.h>

/* Native dependency validation; this is not the user application. */
int main(int argc, char** argv) {
    const struct heif_decoder_descriptor* decoders[4] = {0};
    struct heif_error error = heif_init(NULL);
    if (error.code) { fprintf(stderr, "%s\n", error.message); return 1; }
    int count = heif_get_decoder_descriptors(heif_compression_HEVC, decoders, 4);
    const struct heif_encoder_descriptor* encoder = NULL;
    int encoders = heif_get_encoder_descriptors(heif_compression_HEVC, "x265", &encoder, 1);
    cmsHPROFILE profile = cmsCreate_sRGBProfile();
    if (!profile || count < 1 || encoders != 1 || !encoder ||
        !heif_encoder_descriptor_supports_lossless_compression(encoder)) return 2;
    printf("libheif=%s libde265=%s lcms=%d HEVC_decoders=%d HEVC_encoders=%d encoder=%s lossless=yes\n",
           heif_get_version(), de265_get_version(), cmsGetEncodedCMMversion(), count,
           encoders, heif_encoder_descriptor_get_name(encoder));
    cmsCloseProfile(profile);
    if (argc > 1) {
        struct heif_context* context = heif_context_alloc();
        struct heif_image_handle* handle = NULL;
        struct heif_image* image = NULL;
        heif_context_set_max_decoding_threads(context, 2);
        error = heif_context_read_from_file(context, argv[1], NULL);
        if (error.code) { fprintf(stderr, "%s\n", error.message); return 3; }
        error = heif_context_get_primary_image_handle(context, &handle);
        if (error.code) { fprintf(stderr, "%s\n", error.message); return 4; }
        error = heif_decode_image(handle, &image, heif_colorspace_RGB, heif_chroma_interleaved_RGB, NULL);
        if (error.code) { fprintf(stderr, "%s\n", error.message); return 5; }
        int stride = 0;
        const uint8_t* pixels = heif_image_get_plane_readonly(image, heif_channel_interleaved, &stride);
        if (!pixels || stride < 3) return 6;
        printf("decoded=%dx%d stride=%d first_pixel=%u,%u,%u\n",
               heif_image_get_primary_width(image), heif_image_get_primary_height(image), stride,
               pixels[0], pixels[1], pixels[2]);
        heif_image_release(image);
        heif_image_handle_release(handle);
        heif_context_free(context);
    }
    heif_deinit();
    return 0;
}
