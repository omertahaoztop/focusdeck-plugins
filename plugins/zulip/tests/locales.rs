use std::path::Path;

#[test]
fn locales_match_english() {
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/locales"));
    focusdeck_i18n::check_locales(dir).unwrap();
}
