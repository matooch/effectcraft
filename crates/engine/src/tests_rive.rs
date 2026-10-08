//! File ▸ Import ▸ Rive… through the command layer, on a `.riv` written in the test.

use effectcraft_rive::{Field, RawObject, RawValue, RivFile, keys};
use serde_json::json;

use crate::Session;
use crate::project::ItemId;

fn obj(t: u32, props: &[(u32, RawValue)]) -> RawObject {
    let mut o = RawObject::new(t);
    for (k, v) in props {
        o.set(*k, v.clone());
    }
    o
}

/// One artboard (a shape with a rectangle and a red fill) and two animations of it.
fn sample() -> Vec<u8> {
    use RawValue::{Color, Float, String as S, Uint};
    let f = RivFile {
        major: 7,
        minor: 3,
        file_id: 0,
        toc: vec![] as Vec<(u32, Field)>,
        objects: vec![
            obj(23, &[]),
            obj(1, &[(keys::NAME, S("Board".into())), (keys::ARTBOARD_WIDTH, Float(200.0)), (keys::ARTBOARD_HEIGHT, Float(100.0))]),
            obj(3, &[(keys::NAME, S("Box".into())), (keys::X, Float(50.0)), (keys::Y, Float(50.0))]),
            obj(7, &[(keys::PARENT_ID, Uint(1)), (keys::PATH_WIDTH, Float(20.0)), (keys::PATH_HEIGHT, Float(20.0))]),
            obj(20, &[(keys::PARENT_ID, Uint(1))]),
            obj(18, &[(keys::PARENT_ID, Uint(3)), (keys::SOLID_COLOR, Color(0xffff_0000))]),
            obj(31, &[(keys::ANIMATION_NAME, S("slide".into())), (keys::FPS, Uint(30)), (keys::DURATION, Uint(30))]),
            obj(25, &[(keys::KEYED_OBJECT_ID, Uint(1))]),
            obj(26, &[(keys::KEYED_PROPERTY_KEY, Uint(u64::from(keys::X)))]),
            obj(30, &[(keys::VALUE_DOUBLE, Float(50.0)), (keys::INTERPOLATION_TYPE, Uint(1))]),
            obj(30, &[(keys::FRAME, Uint(30)), (keys::VALUE_DOUBLE, Float(150.0)), (keys::INTERPOLATION_TYPE, Uint(1))]),
            obj(31, &[(keys::ANIMATION_NAME, S("fade".into())), (keys::LOOP, Uint(1))]),
            obj(25, &[(keys::KEYED_OBJECT_ID, Uint(1))]),
            obj(26, &[(keys::KEYED_PROPERTY_KEY, Uint(u64::from(keys::OPACITY)))]),
            obj(30, &[(keys::VALUE_DOUBLE, Float(1.0)), (keys::INTERPOLATION_TYPE, Uint(1))]),
            obj(30, &[(keys::FRAME, Uint(60)), (keys::VALUE_DOUBLE, Float(0.0))]),
        ],
    };
    f.write()
}

#[test]
fn import_rive_makes_a_base_and_linked_timelines() {
    let dir = std::env::temp_dir().join(format!("ec-rive-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sample.riv");
    std::fs::write(&path, sample()).unwrap();
    let mut s = Session::default();
    let before = s.project.items.len();
    let r = s.execute_checked("file.importRive", json!({"path": path.to_string_lossy()})).unwrap();
    let base = ItemId(r["bases"][0].as_u64().unwrap());
    assert_eq!(s.state.active_comp, Some(base));
    let tls: Vec<ItemId> = r["timelines"].as_array().unwrap().iter().map(|t| ItemId(t["comp"].as_u64().unwrap())).collect();
    assert_eq!(tls.len(), 2);
    let slide = s.project.comp(tls[0]).unwrap();
    assert_eq!(slide.timeline_of, Some(base));
    let x = slide.layers[0].props.prop("transform/positionX").unwrap();
    assert_eq!(x.keys.len(), 2);
    // The base and the other timeline are unanimated; editing the base reaches both timelines.
    let lid = slide.layers[0].id.0;
    s.execute("comp.open", json!({"comp": base.0})).unwrap();
    s.execute("layer.rename", json!({"layer": lid, "name": "Hero"})).unwrap();
    for t in &tls {
        assert_eq!(s.project.comp(*t).unwrap().layers[0].name, "Hero");
    }
    assert_eq!(s.project.comp(tls[0]).unwrap().layers[0].props.prop("transform/positionX").unwrap().keys.len(), 2);
    // Undo the rename and the import.
    s.execute("edit.undo", json!({"steps": 1})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.items.len(), before);
    // A file that isn't Rive is an error.
    std::fs::write(&path, b"not rive").unwrap();
    assert!(s.execute("file.importRive", json!({"path": path.to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rive_menu_entry_exists() {
    let entries = crate::menus::entries();
    let e = entries.iter().find(|(_, e)| e.command == "file.importRive").map(|(p, e)| (p.join(" > "), e.label.clone()));
    assert_eq!(e, Some(("File > Import".into(), "Rive...".into())));
}
