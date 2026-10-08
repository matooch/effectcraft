//! Linked timelines through commands: keyframes stay in their timeline, everything else is the
//! base's, and each timeline renders its own animation.

use effectcraft_time::Tick;
use serde_json::json;

use crate::Session;
use crate::project::ItemId;

fn opacity(s: &Session, comp: u64, layer: u64) -> effectcraft_project::Property {
    s.project.comp(ItemId(comp)).unwrap().layer(effectcraft_project::LayerId(layer)).unwrap().props.prop("transform/opacity").unwrap().clone()
}

/// Mean alpha of a frame.
fn coverage(s: &Session, comp: u64, t: f64) -> f32 {
    let img = s.render(ItemId(comp), Tick::from_seconds_f64(t), effectcraft_render::RenderOpts { scale: 0.25, ..Default::default() });
    img.data.iter().map(|p| p[3]).sum::<f32>() / img.data.len().max(1) as f32
}

#[test]
fn timelines_animate_the_base_layers() {
    let mut s = Session::default();
    let base =
        s.execute("comp.new", json!({"name": "Artboard", "width": 320, "height": 240, "frameRate": 30, "duration": 2})).unwrap()["comp"].as_u64().unwrap();
    let solid = s.execute("layer.newSolid", json!({"color": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();

    let fade = s.execute("comp.newLinkedTimeline", json!({"name": "Fade In", "duration": 1})).unwrap();
    let fade = fade["comp"].as_u64().unwrap();
    assert_eq!(s.active_comp_id(), Some(ItemId(fade)));
    // Key opacity 0 → 100 in the timeline, through the ordinary keyframe commands.
    s.execute("prop.toggleAnimation", json!({"layer": solid, "path": "transform/opacity"})).unwrap();
    s.execute("prop.set", json!({"layer": solid, "path": "transform/opacity", "value": 0})).unwrap();
    s.execute("time.set", json!({"time": 0.9})).unwrap();
    s.execute("prop.set", json!({"layer": solid, "path": "transform/opacity", "value": 100})).unwrap();
    assert_eq!(opacity(&s, fade, solid).keys.len(), 2);
    assert!(opacity(&s, base, solid).keys.is_empty());

    // A second timeline of the same base, made from the first (it links to the base).
    let hold = s.execute("comp.newLinkedTimeline", json!({"name": "Hold", "open": false})).unwrap();
    assert_eq!(hold["base"].as_u64(), Some(base));
    let hold = hold["comp"].as_u64().unwrap();

    // Renaming the layer in the timeline renames it everywhere; the keys stay put.
    s.execute("layer.rename", json!({"layer": solid, "name": "Hero"})).unwrap();
    for c in [base, fade, hold] {
        assert_eq!(s.project.comp(ItemId(c)).unwrap().layers[0].name, "Hero");
    }
    assert_eq!(opacity(&s, fade, solid).keys.len(), 2);

    // Each timeline renders its own animation of the shared layer.
    assert!(coverage(&s, fade, 0.0) < 0.01);
    assert!(coverage(&s, fade, 0.95) > 0.99);
    assert!(coverage(&s, hold, 0.0) > 0.99);

    // A layer added to the base reaches both timelines.
    s.execute("comp.open", json!({"comp": base})).unwrap();
    s.execute("layer.newSolid", json!({"color": "#00ff00"})).unwrap();
    for c in [fade, hold] {
        assert_eq!(s.project.comp(ItemId(c)).unwrap().layers.len(), 2, "{c}");
    }

    let listed = s.execute("comp.linkedTimelines", json!({"comp": hold})).unwrap();
    assert_eq!(listed["base"]["id"].as_u64(), Some(base));
    assert_eq!(listed["timelines"].as_array().unwrap().len(), 2);

    // Undo goes back through the synced states in one step each.
    s.execute("edit.undo", json!({})).unwrap();
    for c in [base, fade, hold] {
        assert_eq!(s.project.comp(ItemId(c)).unwrap().layers.len(), 1, "{c}");
    }

    // Unlinked, a timeline keeps its animation and stops following the base.
    s.execute("comp.unlinkTimeline", json!({"comp": fade})).unwrap();
    s.execute("layer.rename", json!({"layer": solid, "name": "Base Only"})).unwrap();
    assert_eq!(s.project.comp(ItemId(fade)).unwrap().layers[0].name, "Hero");
    assert_eq!(s.project.comp(ItemId(hold)).unwrap().layers[0].name, "Base Only");
    assert!(s.execute("comp.unlinkTimeline", json!({"comp": fade})).is_err());
}

#[test]
fn a_base_cannot_nest_its_timeline() {
    let mut s = Session::default();
    let base = s.execute("comp.new", json!({"name": "Artboard", "width": 64, "height": 64, "frameRate": 30, "duration": 1})).unwrap()["comp"].as_u64().unwrap();
    s.execute("layer.newSolid", json!({"color": "#ff0000"})).unwrap();
    let t = s.execute("comp.newLinkedTimeline", json!({"open": false})).unwrap()["comp"].as_u64().unwrap();
    let before = s.project.clone();
    let e = s.execute("layer.addItem", json!({"comp": base, "item": t})).unwrap_err().to_string();
    assert!(e.contains("would contain itself"), "{e}");
    assert!(s.project.same_content(&before));
    assert!(s.execute("comp.newLinkedTimeline", json!({"comp": base, "duration": -1})).is_err());
}
