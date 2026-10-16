fn main() {
    // EmbedFiles: gambar (png) dimasukkan ke dalam .exe, jadi .exe bisa dipindah
    // ke komputer lain tanpa membawa folder ui/.
    let config = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/main.slint", config).unwrap();
}
