//! The user font folder. Its own test binary, because it sets an environment variable.

#[test]
fn user_font_folder_by_file_and_family_name() {
    let system_font = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf";
    if !std::path::Path::new(system_font).exists() {
        eprintln!("skipped: {system_font} not installed");
        return;
    }
    let config = std::env::temp_dir().join(format!("rastercad-fonts-{}", std::process::id()));
    let fonts = config.join("rasterCAD/fonts");
    std::fs::create_dir_all(&fonts).unwrap();
    std::fs::copy(system_font, fonts.join("Custom.ttf")).unwrap();
    // SAFETY: this is the only test in this binary, so no other thread reads the environment.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", &config) };

    assert_eq!(
        rastercad::render::text::user_fonts(),
        vec!["DejaVu Sans:style=Bold".to_string()]
    );
    let out = rastercad::render(
        r#"canvas(80, 40); text("Hi", font="Custom.ttf"); text("Hi", font="DejaVu Sans:style=Bold");"#,
        None,
    );
    assert!(out.image.is_ok(), "{:?}", out.image.err());
    let _ = std::fs::remove_dir_all(&config);
}
