//! Codec and document tests. The `fixtures` tests read the files `fixtures/rive/build.sh` makes
//! with the Rive CLI (not in git); they are skipped when those files are absent.

use std::path::PathBuf;

use crate::model::keys;
use crate::{Document, Field, Interp, Interpolator, KeyValue, Loop, RawObject, RawValue, RivFile, RiveError, property, type_name};

fn fixture(name: &str) -> Option<Vec<u8>> {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "fixtures", "rive", "out", name, &format!("{name}.riv")].iter().collect();
    match std::fs::read(&p) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("skipping: {} not found (run fixtures/rive/build.sh)", p.display());
            None
        }
    }
}

fn sample() -> RivFile {
    let mut ab = RawObject::new(1);
    ab.set(keys::NAME, RawValue::String("Board".into()));
    ab.set(keys::ARTBOARD_WIDTH, RawValue::Float(320.0));
    ab.set(keys::ARTBOARD_HEIGHT, RawValue::Float(200.0));
    let mut shape = RawObject::new(3);
    shape.set(keys::NAME, RawValue::String("Dot".into()));
    shape.set(keys::X, RawValue::Float(10.5));
    shape.set(keys::PARENT_ID, RawValue::Uint(0));
    let mut color = RawObject::new(18);
    color.set(keys::SOLID_COLOR, RawValue::Color(0xff11_2233));
    color.set(keys::PARENT_ID, RawValue::Uint(1));
    // A property from a newer minor version: declared by the table of contents only.
    color.set(60_000, RawValue::Float(2.0));
    let mut anim = RawObject::new(31);
    anim.set(keys::ANIMATION_NAME, RawValue::String("move".into()));
    anim.set(keys::FPS, RawValue::Uint(24));
    let mut ko = RawObject::new(25);
    ko.set(keys::KEYED_OBJECT_ID, RawValue::Uint(1));
    let mut kp = RawObject::new(26);
    kp.set(keys::KEYED_PROPERTY_KEY, RawValue::Uint(u64::from(keys::X)));
    let mut k0 = RawObject::new(30);
    k0.set(keys::VALUE_DOUBLE, RawValue::Float(10.5));
    k0.set(keys::INTERPOLATION_TYPE, RawValue::Uint(1));
    let mut k1 = RawObject::new(30);
    k1.set(keys::FRAME, RawValue::Uint(24));
    k1.set(keys::VALUE_DOUBLE, RawValue::Float(300.0));
    RivFile { major: 7, minor: 3, file_id: 42, toc: vec![(60_000, Field::Float)], objects: vec![RawObject::new(23), ab, shape, color, anim, ko, kp, k0, k1] }
}

#[test]
fn object_model_tables() {
    assert_eq!(property(keys::X), Some(("x", Field::Float)));
    assert_eq!(property(keys::NAME), Some(("name", Field::String)));
    assert_eq!(property(keys::SOLID_COLOR), Some(("colorValue", Field::Color)));
    assert_eq!(property(keys::PATH_ORIGIN_X), Some(("originX", Field::Float)));
    assert_eq!(property(keys::KEYED_PROPERTY_KEY), Some(("propertyKey", Field::Uint)));
    assert_eq!(property(keys::STATE_MACHINE_NAME), Some(("name", Field::String)));
    assert_eq!(type_name(3), Some("Shape"));
    assert_eq!(type_name(28), Some("CubicEaseInterpolator"));
    assert!(crate::in_artboard(3) && crate::in_artboard(28) && !crate::in_artboard(31) && !crate::in_artboard(23));
    for w in crate::defs::PROPS.windows(2) {
        assert!(w[0].0 < w[1].0, "PROPS sorted");
    }
    for w in crate::defs::TYPES.windows(2) {
        assert!(w[0].0 < w[1].0, "TYPES sorted");
    }
}

#[test]
fn write_then_read_is_lossless() {
    let f = sample();
    let bytes = f.write();
    assert_eq!(&bytes[..4], b"RIVE");
    let back = RivFile::read(&bytes).unwrap();
    assert_eq!(back, f);
    assert_eq!(back.write(), bytes);
}

#[test]
fn document_structure_of_a_synthetic_file() {
    let doc = Document::from_file(&sample());
    assert_eq!(doc.artboards.len(), 1);
    let ab = &doc.artboards[0];
    assert_eq!((ab.name.as_str(), ab.width, ab.height), ("Board", 320.0, 200.0));
    assert_eq!(ab.objects.len(), 3);
    assert_eq!(ab.object(1).unwrap().name(), "Dot");
    assert_eq!(ab.children(1), vec![2]);
    let a = &ab.animations[0];
    assert_eq!((a.name.as_str(), a.fps, a.duration, a.looping), ("move", 24, 60, Loop::OneShot));
    let kp = &a.keyed[0].properties[0];
    assert_eq!(kp.key, keys::X);
    assert_eq!(kp.frames.len(), 2);
    assert_eq!(kp.frames[0].interp, Interp::Linear);
    assert_eq!(kp.frames[1].interp, Interp::Hold);
    assert_eq!(kp.frames[1].value, KeyValue::Double(300.0));
    assert_eq!(doc.skipped.get("Backboard"), None);
}

#[test]
fn hostile_input_is_an_error_not_a_crash() {
    assert_eq!(RivFile::read(b""), Err(RiveError::NotRive));
    assert_eq!(RivFile::read(b"RIFF\x07\x00\x00"), Err(RiveError::NotRive));
    assert!(matches!(RivFile::read(b"RIVE\x06\x00\x00\x00"), Err(RiveError::Version { major: 6, .. })));
    // Every truncation of a valid file reads or fails cleanly.
    let bytes = sample().write();
    for n in 0..bytes.len() {
        let _ = RivFile::read(&bytes[..n]);
    }
    // Corrupt every byte in turn.
    for i in 0..bytes.len() {
        for x in [0x00u8, 0x7f, 0x80, 0xff] {
            let mut b = bytes.clone();
            b[i] = x;
            if let Ok(f) = RivFile::read(&b) {
                let _ = Document::from_file(&f);
            }
        }
    }
    // An endless varuint, a huge string length, an unknown property.
    assert!(RivFile::read(b"RIVE\x07\x00\x00\x00\x01\x04\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\x01").is_err());
    assert!(RivFile::read(b"RIVE\x07\x00\x00\x00\x01\x04\xff\xff\xff\xff\x0f").is_err());
    let e = RivFile::read(b"RIVE\x07\x00\x00\x00\x01\xbf\xd4\x03\x01\x00").unwrap_err();
    assert!(e.to_string().contains("Artboard"), "{e}");
}

/// Object counts by type agree with the Rive CLI's `inspect --summary`.
fn check_counts(name: &str, f: &RivFile) {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "fixtures", "rive", "out", name, "summary.txt"].iter().collect();
    let Ok(text) = std::fs::read_to_string(&p) else { return };
    let summary: serde_json::Value = serde_json::from_str(&text).unwrap();
    let want = &summary["artboards"][0]["types"];
    let mut got = std::collections::BTreeMap::<String, u64>::new();
    for o in &f.objects {
        *got.entry(o.type_name().unwrap_or("?").to_string()).or_default() += 1;
    }
    got.remove("Backboard");
    for (t, n) in want.as_object().unwrap() {
        assert_eq!(got.get(t).copied(), n.as_u64(), "{name}: {t}");
    }
    assert_eq!(got.values().sum::<u64>(), summary["artboards"][0]["objects"].as_u64().unwrap(), "{name}");
}

#[test]
fn fixtures_round_trip_byte_for_byte() {
    for name in ["01_basic", "02_easing", "03_multi_anim", "04_hierarchy"] {
        let Some(bytes) = fixture(name) else { continue };
        let f = RivFile::read(&bytes).unwrap();
        assert_eq!((f.major, f.minor), (7, 3), "{name}");
        assert_eq!(f.write(), bytes, "{name}");
        check_counts(name, &f);
        let doc = Document::from_file(&f);
        assert!(doc.skipped.is_empty(), "{name}: {:?}", doc.skipped);
    }
}

#[test]
fn fixture_basic() {
    let Some(bytes) = fixture("01_basic") else { return };
    let doc = Document::read(&bytes).unwrap();
    let ab = &doc.artboards[0];
    assert_eq!((ab.name.as_str(), ab.width, ab.height), ("Basic", 500.0, 500.0));
    assert_eq!(ab.state_machines.len(), 1);
    let a = &ab.animations[0];
    assert_eq!((a.name.as_str(), a.fps, a.duration, a.looping), ("basic", 30, 60, Loop::OneShot));
    let ko = &a.keyed[0];
    let boxed = ab.object(ko.object).unwrap();
    assert_eq!((boxed.type_name().as_str(), boxed.name()), ("Shape", "Box"));
    let rect = ab.object(ab.children(ko.object)[0]).unwrap();
    assert_eq!(rect.type_name(), "Rectangle");
    assert_eq!((rect.float_or(keys::PATH_WIDTH, 0.0), rect.float_or(keys::PATH_HEIGHT, 0.0)), (100.0, 60.0));
    assert_eq!(rect.float_or(keys::PATH_ORIGIN_X, 0.5), 0.5);
    let prop = |k: u32| ko.properties.iter().find(|p| p.key == k).unwrap();
    let frames = |k: u32| prop(k).frames.iter().map(|f| (f.frame, f.value.clone())).collect::<Vec<_>>();
    assert_eq!(frames(keys::X), vec![(0, KeyValue::Double(150.0)), (60, KeyValue::Double(350.0))]);
    assert_eq!(frames(keys::ROTATION).len(), 3);
    assert_eq!(frames(keys::ROTATION)[1].0, 30);
    assert_eq!(frames(keys::SCALE_Y)[1], (60, KeyValue::Double(0.5)));
    assert_eq!(frames(keys::OPACITY)[1], (60, KeyValue::Double(0.25)));
    assert!(ko.properties.iter().all(|p| p.frames.iter().all(|f| f.interp == Interp::Linear)));
}

#[test]
fn fixture_easing() {
    let Some(bytes) = fixture("02_easing") else { return };
    let doc = Document::read(&bytes).unwrap();
    let ab = &doc.artboards[0];
    let a = &ab.animations[0];
    let by_name = |n: &str| a.keyed.iter().find(|k| ab.object(k.object).unwrap().name() == n).unwrap();
    let first = |n: &str| by_name(n).properties[0].frames[0].clone();
    assert_eq!(first("Linear").interp, Interp::Linear);
    let ease = first("EaseInOut");
    assert_eq!(ease.interp, Interp::Cubic);
    assert_eq!(ab.interpolator(ease.interpolator.unwrap()), Some(Interpolator::Cubic { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 }));
    let custom = first("Custom");
    let Some(Interpolator::Cubic { x1, y1, x2, y2 }) = ab.interpolator(custom.interpolator.unwrap()) else { panic!() };
    assert!((x1 - 0.1).abs() < 1e-6 && (y1 - 0.9).abs() < 1e-6 && (x2 - 0.2).abs() < 1e-6 && y2 == 1.0);
    let hold = &by_name("Hold").properties[0].frames;
    assert_eq!(hold.len(), 3);
    assert!(hold.iter().all(|f| f.interp == Interp::Hold));
}

#[test]
fn fixture_multiple_animations() {
    let Some(bytes) = fixture("03_multi_anim") else { return };
    let doc = Document::read(&bytes).unwrap();
    let ab = &doc.artboards[0];
    let got: Vec<_> = ab.animations.iter().map(|a| (a.name.as_str(), a.fps, a.duration, a.looping)).collect();
    assert_eq!(got, vec![("idle", 60, 60, Loop::Loop), ("bounce", 30, 30, Loop::PingPong), ("spin", 24, 48, Loop::OneShot)]);
    let ball = ab.object(ab.animations[1].keyed[0].object).unwrap();
    assert_eq!(ball.name(), "Ball");
    assert_eq!(ab.object(ab.children(ab.animations[1].keyed[0].object)[0]).unwrap().type_name(), "Ellipse");
}

#[test]
fn fixture_hierarchy() {
    let Some(bytes) = fixture("04_hierarchy") else { return };
    let doc = Document::read(&bytes).unwrap();
    let ab = &doc.artboards[0];
    let find = |n: &str| ab.objects.iter().position(|o| o.name() == n).unwrap() as u64;
    let (arm, upper, elbow, hand) = (find("Arm"), find("Upper"), find("Elbow"), find("Hand"));
    assert_eq!(ab.object(arm).unwrap().parent(), 0);
    assert_eq!(ab.object(upper).unwrap().parent(), arm);
    assert_eq!(ab.object(elbow).unwrap().parent(), arm);
    assert_eq!(ab.object(hand).unwrap().parent(), elbow);
    assert_eq!(ab.object(elbow).unwrap().float_or(keys::X, 0.0), 150.0);
    let bar = ab.object(find("Bar")).unwrap();
    assert_eq!((bar.float_or(keys::PATH_ORIGIN_X, 0.5), bar.float_or(keys::PATH_ORIGIN_Y, 0.5)), (0.0, 0.5));
    let a = &ab.animations[0];
    assert_eq!(a.keyed.iter().map(|k| k.object).collect::<Vec<_>>(), vec![arm, hand]);
}
