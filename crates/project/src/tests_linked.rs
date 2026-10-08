//! Linked timelines ([`crate::linked`]).

use effectcraft_keyframe::{Keyframe, Value};
use effectcraft_time::{FrameRate, Tick};

use crate::build;
use crate::linked::{self, sync_project};
use crate::{Comp, ItemId, ItemKind, LayerId, LayerSource, Project, Solid};

fn secs(s: f64) -> Tick {
    Tick::from_seconds_f64(s)
}

/// A base comp with two solid layers.
fn base_project() -> (Project, ItemId) {
    let mut p = Project::default();
    let comp = Comp::new(800, 600, FrameRate::FPS_30, secs(4.0));
    let cid = p.add_item("Artboard", effectcraft_color::Label::Sandstone, None, ItemKind::Comp(comp.clone().into()));
    let sid =
        p.add_item("Solid", effectcraft_color::Label::Red, None, ItemKind::Solid(Solid { color: [1.0, 0.0, 0.0], width: 100, height: 100, pixel_aspect: 1.0 }));
    for name in ["Box", "Circle"] {
        let l = build::layer(&mut p, &comp, name, LayerSource::Solid { item: sid }, (100, 100), None);
        p.comp_mut(cid).unwrap().layers.push(l);
    }
    (p, cid)
}

/// Apply `f` the way the engine applies an edit: on a copy, then synced.
fn edit(p: &mut Project, f: impl FnOnce(&mut Project)) -> Result<(), crate::ProjectError> {
    let before = p.clone();
    let mut after = p.clone();
    f(&mut after);
    sync_project(&before, &mut after)?;
    *p = after;
    Ok(())
}

fn layer_id(p: &Project, comp: ItemId, i: usize) -> LayerId {
    p.comp(comp).unwrap().layers[i].id
}

fn opacity(p: &Project, comp: ItemId, layer: LayerId) -> &crate::Property {
    p.comp(comp).unwrap().layer(layer).unwrap().props.prop("transform/opacity").unwrap()
}

fn key_opacity(p: &mut Project, comp: ItemId, layer: LayerId, keys: &[(f64, f64)]) {
    let pr = p.comp_mut(comp).unwrap().layer_mut(layer).unwrap().props.prop_mut("transform/opacity").unwrap();
    pr.keys = keys.iter().map(|(t, v)| Keyframe::new(secs(*t), Value::Scalar(*v))).collect();
}

#[test]
fn new_timeline_mirrors_the_base_unanimated() {
    let (mut p, base) = base_project();
    let l = layer_id(&p, base, 0);
    // The base's own animation is its rest pose at time 0 in timelines.
    key_opacity(&mut p, base, l, &[(0.0, 40.0), (1.0, 100.0)]);
    let t = linked::new_timeline(&mut p, base, "Idle", Some(secs(2.0))).unwrap();
    let tc = p.comp(t).unwrap();
    assert_eq!(tc.timeline_of, Some(base));
    assert_eq!((tc.width, tc.height, tc.duration, tc.frame_rate), (800, 600, secs(2.0), FrameRate::FPS_30));
    assert_eq!(tc.layers.iter().map(|l| l.id).collect::<Vec<_>>(), p.comp(base).unwrap().layers.iter().map(|l| l.id).collect::<Vec<_>>());
    let o = opacity(&p, t, l);
    assert!(o.keys.is_empty());
    assert_eq!(o.value, Value::Scalar(40.0));
    assert_eq!(linked::base_of(&p, t), Some(base));
    assert_eq!(linked::timelines_of(&p, base), vec![t]);
    // A timeline of a timeline links to the base.
    let t2 = linked::new_timeline(&mut p, t, "Run", None).unwrap();
    assert_eq!(p.comp(t2).unwrap().timeline_of, Some(base));
    assert_eq!(p.comp(t2).unwrap().duration, secs(4.0));
}

#[test]
fn keyframes_belong_to_their_timeline() {
    let (mut p, base) = base_project();
    let l = layer_id(&p, base, 0);
    let idle = linked::new_timeline(&mut p, base, "Idle", None).unwrap();
    let run = linked::new_timeline(&mut p, base, "Run", None).unwrap();
    edit(&mut p, |p| key_opacity(p, idle, l, &[(0.0, 0.0), (1.0, 100.0)])).unwrap();
    assert_eq!(opacity(&p, idle, l).keys.len(), 2);
    assert!(opacity(&p, base, l).keys.is_empty() && opacity(&p, base, l).value == Value::Scalar(100.0));
    assert!(opacity(&p, run, l).keys.is_empty());
    // Keyframes survive base edits.
    edit(&mut p, |p| p.comp_mut(base).unwrap().layer_mut(l).unwrap().name = "Hero".into()).unwrap();
    assert_eq!(p.comp(idle).unwrap().layer(l).unwrap().name, "Hero");
    assert_eq!(opacity(&p, idle, l).keys.len(), 2);
}

#[test]
fn static_edits_in_a_timeline_go_to_the_base_and_its_other_timelines() {
    let (mut p, base) = base_project();
    let l = layer_id(&p, base, 1);
    let idle = linked::new_timeline(&mut p, base, "Idle", None).unwrap();
    let run = linked::new_timeline(&mut p, base, "Run", None).unwrap();
    edit(&mut p, |p| key_opacity(p, run, l, &[(0.0, 10.0), (2.0, 90.0)])).unwrap();
    // An unanimated value and a rename made in Idle.
    edit(&mut p, |p| {
        let c = p.comp_mut(idle).unwrap();
        let layer = c.layer_mut(l).unwrap();
        layer.name = "Ball".into();
        layer.props.prop_mut("transform/rotation").unwrap().value = Value::Scalar(45.0);
    })
    .unwrap();
    for c in [base, idle, run] {
        let layer = p.comp(c).unwrap().layer(l).unwrap();
        assert_eq!(layer.name, "Ball");
        assert_eq!(layer.props.prop("transform/rotation").unwrap().value, Value::Scalar(45.0));
    }
    // Run kept its animation; nothing went up.
    assert_eq!(opacity(&p, run, l).keys.len(), 2);
    assert!(opacity(&p, base, l).keys.is_empty());
}

#[test]
fn keying_or_unkeying_in_a_timeline_leaves_the_base_alone() {
    let (mut p, base) = base_project();
    let l = layer_id(&p, base, 0);
    let t = linked::new_timeline(&mut p, base, "Idle", None).unwrap();
    edit(&mut p, |p| key_opacity(p, t, l, &[(0.0, 20.0), (1.0, 60.0)])).unwrap();
    // Stopwatch off at 1 s: the value it leaves is not a base edit; the base value shows again.
    edit(&mut p, |p| p.comp_mut(t).unwrap().layer_mut(l).unwrap().props.prop_mut("transform/opacity").unwrap().set_animated(false, secs(1.0))).unwrap();
    assert_eq!(opacity(&p, base, l).value, Value::Scalar(100.0));
    assert_eq!(opacity(&p, t, l).value, Value::Scalar(100.0));
    assert!(opacity(&p, t, l).keys.is_empty());
}

#[test]
fn layers_added_or_deleted_anywhere_reach_every_timeline() {
    let (mut p, base) = base_project();
    let idle = linked::new_timeline(&mut p, base, "Idle", None).unwrap();
    let run = linked::new_timeline(&mut p, base, "Run", None).unwrap();
    let first = layer_id(&p, base, 0);
    // Added in a timeline, with keys: the base gets it unanimated, the timeline keeps its keys.
    let added = {
        let comp = p.comp(idle).unwrap().clone();
        let sid = p.find_by_name("Solid").unwrap().id;
        build::layer(&mut p, &comp, "New", LayerSource::Solid { item: sid }, (50, 50), None)
    };
    let nid = added.id;
    edit(&mut p, |p| {
        p.comp_mut(idle).unwrap().layers.insert(0, added);
        key_opacity(p, idle, nid, &[(0.0, 0.0), (1.0, 100.0)]);
    })
    .unwrap();
    for c in [base, idle, run] {
        assert_eq!(layer_id(&p, c, 0), nid, "{c:?}");
        assert_eq!(p.comp(c).unwrap().layers.len(), 3);
    }
    assert_eq!(opacity(&p, idle, nid).keys.len(), 2);
    assert!(opacity(&p, base, nid).keys.is_empty() && opacity(&p, run, nid).keys.is_empty());
    assert_eq!(opacity(&p, base, nid).value, Value::Scalar(0.0));
    // Deleted in another timeline: gone everywhere.
    edit(&mut p, |p| p.comp_mut(run).unwrap().layers.retain(|l| l.id != first)).unwrap();
    for c in [base, idle, run] {
        assert!(p.comp(c).unwrap().layer(first).is_none(), "{c:?}");
        assert_eq!(p.comp(c).unwrap().layers.len(), 2);
    }
    // Added to the base: reaches the timelines unanimated.
    let from_base = {
        let comp = p.comp(base).unwrap().clone();
        let sid = p.find_by_name("Solid").unwrap().id;
        build::layer(&mut p, &comp, "Base Only", LayerSource::Solid { item: sid }, (50, 50), None)
    };
    let bid = from_base.id;
    edit(&mut p, |p| p.comp_mut(base).unwrap().layers.push(from_base)).unwrap();
    assert_eq!(p.comp(idle).unwrap().layers.last().unwrap().id, bid);
    assert_eq!(p.comp(run).unwrap().layers.last().unwrap().id, bid);
}

#[test]
fn frame_size_is_shared_and_timing_is_not() {
    let (mut p, base) = base_project();
    let t = linked::new_timeline(&mut p, base, "Idle", None).unwrap();
    edit(&mut p, |p| {
        let c = p.comp_mut(t).unwrap();
        c.width = 1024;
        c.duration = secs(1.5);
    })
    .unwrap();
    assert_eq!(p.comp(base).unwrap().width, 1024);
    assert_eq!(p.comp(base).unwrap().duration, secs(4.0));
    edit(&mut p, |p| p.comp_mut(base).unwrap().height = 512).unwrap();
    assert_eq!((p.comp(t).unwrap().width, p.comp(t).unwrap().height), (1024, 512));
    assert_eq!(p.comp(t).unwrap().duration, secs(1.5));
}

#[test]
fn a_timeline_whose_base_is_gone_becomes_an_ordinary_comp() {
    let (mut p, base) = base_project();
    let l = layer_id(&p, base, 0);
    let t = linked::new_timeline(&mut p, base, "Idle", None).unwrap();
    edit(&mut p, |p| key_opacity(p, t, l, &[(0.0, 0.0), (1.0, 100.0)])).unwrap();
    edit(&mut p, |p| {
        p.items.remove(&base);
    })
    .unwrap();
    let tc = p.comp(t).unwrap();
    assert_eq!(tc.timeline_of, None);
    assert_eq!(tc.layers.len(), 2);
    assert_eq!(opacity(&p, t, l).keys.len(), 2);
}

#[test]
fn a_base_cannot_nest_its_own_timeline() {
    let (mut p, base) = base_project();
    let t = linked::new_timeline(&mut p, base, "Idle", None).unwrap();
    let pre = {
        let comp = p.comp(base).unwrap().clone();
        build::layer(&mut p, &comp, "Idle", LayerSource::Comp { item: t }, (800, 600), None)
    };
    let before = p.clone();
    let e = edit(&mut p, |p| p.comp_mut(base).unwrap().layers.push(pre)).unwrap_err();
    assert!(e.to_string().contains("would contain itself"), "{e}");
    assert!(p.same_content(&before));
}

#[test]
fn comp_contains_survives_a_nesting_cycle() {
    // Regression: a cyclic project (two comps nesting each other) recursed without end.
    let (mut p, a) = base_project();
    let b = p.add_item("B", effectcraft_color::Label::Sandstone, None, ItemKind::Comp(Comp::new(10, 10, FrameRate::FPS_30, secs(1.0)).into()));
    let ca = p.comp(a).unwrap().clone();
    let la = build::layer(&mut p, &ca, "B", LayerSource::Comp { item: b }, (10, 10), None);
    let lb = build::layer(&mut p, &ca, "A", LayerSource::Comp { item: a }, (10, 10), None);
    p.comp_mut(a).unwrap().layers.push(la);
    p.comp_mut(b).unwrap().layers.push(lb);
    let c = p.add_item("C", effectcraft_color::Label::Sandstone, None, ItemKind::Comp(Comp::new(10, 10, FrameRate::FPS_30, secs(1.0)).into()));
    assert!(!p.comp_contains(a, c));
    assert!(p.comp_contains(a, b));
}

#[test]
fn the_link_is_saved() {
    let (mut p, base) = base_project();
    let t = linked::new_timeline(&mut p, base, "Idle", None).unwrap();
    let back = Project::from_json(&p.to_json()).unwrap();
    assert_eq!(back.comp(t).unwrap().timeline_of, Some(base));
    assert!(!p.to_json().contains("\"timeline_of\": null"));
    assert!(back.comp(base).unwrap().timeline_of.is_none());
}
