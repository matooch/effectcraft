//! Import tests: structure of the imported comps, and frames rendered by EffectCraft compared
//! with the Rive CLI's captures (`fixtures/rive/build.sh`; skipped when absent).

use std::path::PathBuf;

use effectcraft_project::{ItemId, Project, Value};
use effectcraft_time::Tick;

use crate::import;

fn out_dir(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "fixtures", "rive", "out", name].iter().collect()
}

fn load(name: &str) -> Option<(Project, crate::ImportResult)> {
    let bytes = std::fs::read(out_dir(name).join(format!("{name}.riv"))).ok()?;
    let mut p = Project::default();
    let r = import(&mut p, &bytes, name).unwrap();
    Some((p, r))
}

/// Mean absolute difference (0–255) between our frame and Rive's capture, and the share of
/// pixels that differ by more than 64 in some channel.
fn compare(p: &Project, comp: ItemId, t: f64, reference: &PathBuf, save: Option<&str>) -> Option<(f64, f64)> {
    let refimg = image::open(reference).ok()?.to_rgba8();
    let c = p.comp(comp)?;
    let scale = refimg.width() as f64 / c.width as f64;
    let img = effectcraft_render::render_frame(p, comp, Tick::from_seconds_f64(t), scale);
    let ours = img.to_rgba8_over(c.background);
    if let Some(path) = save {
        let _ = image::save_buffer(path, &ours, img.width, img.height, image::ExtendedColorType::Rgba8);
    }
    if (img.width, img.height) != (refimg.width(), refimg.height()) {
        return Some((255.0, 1.0));
    }
    let (mut sum, mut bad) = (0u64, 0u64);
    for (a, b) in ours.chunks(4).zip(refimg.as_raw().chunks(4)) {
        let mut worst = 0;
        for ch in 0..3 {
            let d = (i32::from(a[ch]) - i32::from(b[ch])).unsigned_abs();
            sum += u64::from(d);
            worst = worst.max(d);
        }
        if worst > 64 {
            bad += 1;
        }
    }
    let n = (ours.len() / 4) as f64;
    Some((sum as f64 / (n * 3.0), bad as f64 / n))
}

/// Compare the first animation's timeline with Rive's captures at 0.5, 1, 1.5 and 2 s.
fn check_frames(name: &str) {
    let Some((p, r)) = load(name) else {
        eprintln!("skipping {name}: run fixtures/rive/build.sh");
        return;
    };
    let (tl, _) = r.timelines[0];
    let c = p.comp(tl).unwrap();
    let dur = c.duration.seconds();
    let last = dur - c.frame_rate.frame_duration().seconds();
    let looping = p.item(tl).unwrap().comment.contains("loop");
    let dir = std::env::var("RIVE_COMPARE_OUT").ok();
    for ms in [500u32, 1000, 1500, 2000] {
        let reference = out_dir(name).join(format!("t{ms:04}ms.png"));
        if !reference.exists() {
            continue;
        }
        // The CLI's first 1/60 s advance only enters the state machine, so its capture at
        // `ms` shows the animation one display frame earlier.
        let t = ms as f64 / 1000.0 - 1.0 / 60.0;
        let t = if looping { t % dur } else { t.min(last) };
        let save = dir.as_ref().map(|d| format!("{d}/{name}_{ms}ms_ours.png"));
        let (mean, bad) = compare(&p, tl, t, &reference, save.as_deref()).unwrap();
        eprintln!("{name} @{ms}ms: mean diff {mean:.2}, differing pixels {:.2}%", bad * 100.0);
        assert!(mean < 0.5 && bad < 0.0025, "{name} @{ms}ms: mean {mean:.2}, bad {:.2}%", bad * 100.0);
    }
}

#[test]
fn basic_structure() {
    let Some((p, r)) = load("01_basic") else { return };
    assert_eq!(r.bases.len(), 1);
    assert_eq!(r.timelines.len(), 1);
    let base = p.comp(r.bases[0]).unwrap();
    assert_eq!((base.width, base.height), (500, 500));
    assert_eq!(base.layers.len(), 1);
    let (tl, b) = r.timelines[0];
    assert_eq!(b, r.bases[0]);
    assert_eq!(p.item(tl).unwrap().name, "basic");
    let c = p.comp(tl).unwrap();
    assert_eq!(c.timeline_of, Some(r.bases[0]));
    assert_eq!(c.frame_rate.as_f64(), 30.0);
    assert!((c.duration.seconds() - 2.0).abs() < 1e-6);
    let tr = c.layers[0].props.sub("transform").unwrap();
    let x = tr.get("positionX").unwrap();
    assert_eq!(x.keys.len(), 2);
    assert_eq!(x.keys[1].value, Value::Scalar(350.0));
    let rot = tr.get("rotation").unwrap();
    assert_eq!(rot.keys.len(), 3);
    assert!((rot.keys[2].value.as_f64() - 90.0).abs() < 1e-4);
    let sc = tr.get("scale").unwrap();
    assert_eq!(sc.keys.len(), 2);
    assert_eq!(sc.keys[1].value, Value::Vec3([150.0, 50.0, 100.0]));
    // The base stays unanimated.
    assert!(base.layers[0].props.sub("transform").unwrap().get("positionX").unwrap().keys.is_empty());
}

#[test]
fn multi_animation_structure() {
    let Some((p, r)) = load("03_multi_anim") else { return };
    let names: Vec<_> = r.timelines.iter().map(|(t, _)| p.item(*t).unwrap().name.clone()).collect();
    assert_eq!(names, ["idle", "bounce", "spin"]);
    let rates: Vec<_> = r.timelines.iter().map(|(t, _)| p.comp(*t).unwrap().frame_rate.as_f64()).collect();
    assert_eq!(rates, [60.0, 30.0, 24.0]);
    assert!(p.item(r.timelines[1].0).unwrap().comment.contains("ping-pong"));
    assert!(r.warnings.iter().any(|w| w.contains("state machine")));
    assert_eq!(effectcraft_project::linked::timelines_of(&p, r.bases[0]).len(), 3);
}

#[test]
fn hierarchy_structure() {
    let Some((p, r)) = load("04_hierarchy") else { return };
    let base = p.comp(r.bases[0]).unwrap();
    let id = |n: &str| base.layer_by_name(n).unwrap().id;
    assert_eq!(base.layer_by_name("Upper").unwrap().parent, Some(id("Arm")));
    assert_eq!(base.layer_by_name("Hand").unwrap().parent, Some(id("Elbow")));
    assert_eq!(base.layer_by_name("Elbow").unwrap().parent, Some(id("Arm")));
}

#[test]
fn frames_match_rive_basic() {
    check_frames("01_basic");
}

#[test]
fn frames_match_rive_easing() {
    check_frames("02_easing");
}

#[test]
fn frames_match_rive_multi_animation() {
    check_frames("03_multi_anim");
}

#[test]
fn frames_match_rive_hierarchy() {
    check_frames("04_hierarchy");
}
