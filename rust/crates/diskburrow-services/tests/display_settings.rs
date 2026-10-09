use diskburrow_services::{AppSettings, SettingsStore};
use serde_json::{Value, json};
use std::fs;

#[test]
fn legacy_document_keeps_choices_and_gets_display_defaults() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("settings.json"),
        r#"{"Language":"en","Theme":"dark","IntervalHours":12,"Paused":true,"ExcludedPaths":["E:\\cache"]}"#,
    )
    .unwrap();
    let mut store = SettingsStore::new(dir.path());
    let settings = store.load();
    assert!(store.last_user_message.is_none());
    let document = serde_json::to_value(settings).unwrap();
    assert_eq!(document["Language"], "en");
    assert_eq!(document["Theme"], "dark");
    assert_eq!(document["IntervalHours"], 12);
    assert_eq!(document["Paused"], true);
    assert_eq!(document["ExcludedPaths"], json!([r"E:\cache"]));
    assert_eq!(document["UiScalePercent"], 100);
    assert_eq!(document["MapDepth"], 3);
    assert_eq!(document["ShowHidden"], true);
    assert_eq!(document["SidebarWidth"], 225);
}

#[test]
fn display_choices_survive_store_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SettingsStore::new(dir.path());
    let requested = json!({
        "Theme": "system",
        "UiScalePercent": 150,
        "MapDepth": 6,
        "ShowHidden": false,
        "SidebarWidth": 420,
    });
    let settings: AppSettings = serde_json::from_value(requested.clone()).unwrap();
    store.save(&settings).unwrap();
    let loaded = serde_json::to_value(store.load()).unwrap();
    for (field, expected) in requested.as_object().unwrap() {
        assert_eq!(&loaded[field], expected, "{field}");
    }
}

#[test]
fn valid_display_boundaries_and_scale_steps_are_accepted() {
    for (field, values) in [
        ("UiScalePercent", vec![75, 90, 100, 110, 125, 150]),
        ("MapDepth", vec![1, 2, 3, 4, 5, 6]),
        ("SidebarWidth", vec![180, 225, 420]),
    ] {
        for value in values {
            let settings: AppSettings = serde_json::from_value(json!({field: value})).unwrap();
            assert!(settings.validate().is_ok(), "{field}={value}");
        }
    }
    for theme in ["light", "dark", "system"] {
        let settings: AppSettings = serde_json::from_value(json!({"Theme": theme})).unwrap();
        assert!(settings.validate().is_ok(), "{theme}");
    }
}

#[test]
fn invalid_display_values_are_rejected_without_overwriting_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SettingsStore::new(dir.path());
    store.save(&AppSettings::default()).unwrap();
    let before = fs::read(dir.path().join("settings.json")).unwrap();
    for (field, values) in [
        (
            "UiScalePercent",
            vec![0, 74, 76, 89, 91, 99, 101, 124, 126, 149, 151, 65535],
        ),
        ("MapDepth", vec![0, 7, 255]),
        ("SidebarWidth", vec![0, 179, 421, 65535]),
    ] {
        for value in values {
            let settings: AppSettings = serde_json::from_value(json!({field: value})).unwrap();
            assert!(store.save(&settings).is_err(), "{field}={value}");
            assert_eq!(fs::read(dir.path().join("settings.json")).unwrap(), before);
        }
    }
}

#[test]
fn malformed_display_fields_are_not_silently_ignored() {
    for (field, values) in [
        (
            "UiScalePercent",
            vec![json!(-1), json!(65536), Value::Null, json!("100")],
        ),
        (
            "MapDepth",
            vec![json!(-1), json!(256), Value::Null, json!(1.5)],
        ),
        ("ShowHidden", vec![Value::Null, json!(0), json!("false")]),
        ("SidebarWidth", vec![json!(-1), json!(65536), Value::Null]),
    ] {
        for value in values {
            assert!(
                serde_json::from_value::<AppSettings>(json!({field: value})).is_err(),
                "{field} accepted malformed field"
            );
        }
    }
}
