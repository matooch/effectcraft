//! `.riv` → EffectCraft: each artboard becomes a base composition, each of its linear
//! animations a linked timeline of that base (`effectcraft_project::linked`).
//!
//! - Nodes become null layers and shapes become shape layers, with Rive's hierarchy as layer
//!   parenting. Every layer's anchor point is its origin, so Rive's transform (translate,
//!   rotate, scale about the origin) maps one to one: Position (separated X/Y), Rotation
//!   (radians → degrees), Scale (×100), Opacity (×100).
//! - A shape's rectangles and ellipses become shape paths (Rive's `originX`/`originY` move the
//!   path's centre), its solid fills and strokes become Fill and Stroke.
//! - Keyframes keep their frames (at the animation's frame rate, which becomes the timeline's)
//!   and their interpolation: hold, linear, and cubic curves as Bezier speed/influence. Vector
//!   properties keyed per channel in Rive (scale X/Y, width/height, colour) merge into one
//!   property when the channels share their keys, and are baked per frame otherwise.
//! - What has no counterpart (state machines, bones, constraints, images, text, gradients…) is
//!   listed in [`ImportResult::warnings`].

use std::collections::BTreeMap;
use std::sync::Arc;

use effectcraft_color::Label;
use effectcraft_project::build::{self, Ids};
use effectcraft_project::keyframe::{Ease, Interp as EcInterp};
use effectcraft_project::props::{Node as PropNode, ParamUi, Property};
use effectcraft_project::{Comp, ItemId, ItemKind, Keyframe, LayerId, LayerSource, Project, Uid, Value, linked};
use effectcraft_time::{FrameRate, Tick};

use crate::model::keys;
use crate::{Artboard, Document, Interp, Interpolator, KeyFrame, KeyValue, LinearAnimation, Loop, RiveError};

/// What an import made.
#[derive(Clone, Debug, Default)]
pub struct ImportResult {
    /// The folder holding everything.
    pub folder: Option<ItemId>,
    /// One base composition per artboard.
    pub bases: Vec<ItemId>,
    /// Linked timelines, one per linear animation (with the base each belongs to).
    pub timelines: Vec<(ItemId, ItemId)>,
    pub warnings: Vec<String>,
}

mod ty {
    pub const NODE: u32 = 2;
    pub const SHAPE: u32 = 3;
    pub const ELLIPSE: u32 = 4;
    pub const RECTANGLE: u32 = 7;
    pub const SOLID_COLOR: u32 = 18;
    pub const FILL: u32 = 20;
    pub const STROKE: u32 = 24;
}

const STROKE_THICKNESS: u32 = 47;
const DEPTH_LIMIT: usize = 256;

/// Import a `.riv` file into `project`. `fallback_name` names the folder when the file
/// doesn't (it never does: Rive files carry no title).
pub fn import(project: &mut Project, bytes: &[u8], fallback_name: &str) -> Result<ImportResult, RiveError> {
    let doc = Document::read(bytes)?;
    if doc.artboards.is_empty() {
        return Err(RiveError::Malformed("the file has no artboards".into()));
    }
    let mut res = ImportResult::default();
    let folder = project.add_item(fallback_name, Label::None, None, ItemKind::Folder);
    res.folder = Some(folder);
    for (what, n) in &doc.skipped {
        res.warnings.push(format!("{n} × {what} not imported"));
    }
    for ab in &doc.artboards {
        import_artboard(project, ab, folder, &mut res);
    }
    project.fix_next_id();
    Ok(res)
}

/// Where a Rive object's properties went.
#[derive(Clone, Copy)]
enum Target {
    /// A layer and what its keyed x/y add (the artboard origin, for top-level layers).
    Layer(LayerId, [f64; 2]),
    /// A rectangle or ellipse: its layer, Size and Position, and its Rive origin.
    Path { layer: LayerId, size: Uid, position: Uid, offset: [f64; 2], origin: [f64; 2] },
    /// A solid colour: its layer and the Fill/Stroke Color.
    Color { layer: LayerId, color: Uid },
}

struct Built {
    targets: BTreeMap<u64, Target>,
}

fn rate_of(fps: u64) -> FrameRate {
    FrameRate::new(fps.clamp(1, 1000) as i64, 1)
}

fn frames_to_tick(frame: f64, fps: u64) -> Tick {
    Tick::from_seconds_f64(frame / fps.clamp(1, 1000) as f64)
}

fn argb(c: u32) -> [f64; 4] {
    let ch = |s: u32| f64::from((c >> s) & 0xff) / 255.0;
    [ch(16), ch(8), ch(0), ch(24)]
}

fn import_artboard(project: &mut Project, ab: &Artboard, folder: ItemId, res: &mut ImportResult) {
    let warn = |res: &mut ImportResult, w: String| {
        if !res.warnings.contains(&w) {
            res.warnings.push(w);
        }
    };
    let fps = ab.animations.first().map(|a| a.fps).unwrap_or(60).clamp(1, 1000);
    let rate = rate_of(fps);
    let longest = ab.animations.iter().map(|a| a.duration as f64 / a.fps.clamp(1, 1000) as f64).fold(0.0f64, f64::max);
    let duration = rate.snap_nearest(Tick::from_seconds_f64(if longest > 0.0 { longest } else { 1.0 })).max(rate.frame_duration());
    let (w, h) = (ab.width.round().clamp(1.0, 30000.0) as u32, ab.height.round().clamp(1.0, 30000.0) as u32);
    let mut comp = Comp::new(w, h, rate, duration);
    let root = ab.object(0);
    let origin =
        root.map(|o| (o.float_or(keys::ARTBOARD_ORIGIN_X, 0.0) * ab.width, o.float_or(keys::ARTBOARD_ORIGIN_Y, 0.0) * ab.height)).unwrap_or((0.0, 0.0));
    // The artboard's own solid fill is the background.
    for c in ab.children(0) {
        if ab.object(c).is_some_and(|o| o.type_key() == ty::FILL)
            && let Some(sc) = ab.children(c).into_iter().filter_map(|i| ab.object(i)).find(|o| o.type_key() == ty::SOLID_COLOR)
        {
            let [r, g, b, _] = argb(sc.raw.color(keys::SOLID_COLOR).unwrap_or(0xff74_7474));
            comp.background = [r as f32, g as f32, b as f32];
        }
    }

    // Layers in file order: Rive draws earlier siblings in front (the top of its hierarchy
    // panel), which is the top of the layer stack.
    let mut built = Built { targets: BTreeMap::new() };
    let mut layers = Vec::new();
    let mut layer_of: BTreeMap<u64, LayerId> = BTreeMap::new();
    for (i, o) in ab.objects.iter().enumerate().skip(1) {
        let i = i as u64;
        let t = o.type_key();
        if t != ty::NODE && t != ty::SHAPE {
            continue;
        }
        let name = if o.name().is_empty() { o.type_name() } else { o.name().to_string() };
        let source = if t == ty::SHAPE { LayerSource::Shape } else { LayerSource::Null };
        let mut layer = build::layer(project, &comp, &name, source, (w, h), None);
        // Parent: the nearest ancestor that is a layer.
        let mut p = o.parent();
        for _ in 0..DEPTH_LIMIT {
            if p == 0 {
                break;
            }
            if let Some(l) = layer_of.get(&p) {
                layer.parent = Some(*l);
                break;
            }
            p = ab.object(p).map(|x| x.parent()).unwrap_or(0);
        }
        let top = layer.parent.is_none();
        let (x, y) = (o.float_or(keys::X, 0.0), o.float_or(keys::Y, 0.0));
        let (x, y) = if top { (x + origin.0, y + origin.1) } else { (x, y) };
        if let Some(tr) = layer.props.sub_mut("transform") {
            if let Some(a) = tr.get_mut("anchor") {
                a.value = Value::Vec3([0.0; 3]);
            }
            separate_position(tr, [x, y], &mut project.next_id);
            if let Some(r) = tr.get_mut("rotation") {
                r.value = Value::Scalar(o.float_or(keys::ROTATION, 0.0).to_degrees());
            }
            if let Some(s) = tr.get_mut("scale") {
                s.value = Value::Vec3([o.float_or(keys::SCALE_X, 1.0) * 100.0, o.float_or(keys::SCALE_Y, 1.0) * 100.0, 100.0]);
            }
            if let Some(op) = tr.get_mut("opacity") {
                op.value = Value::Scalar(o.float_or(keys::OPACITY, 1.0) * 100.0);
            }
        }
        if t == ty::SHAPE {
            shape_contents(project, ab, i, &mut layer, &mut built, res);
        }
        layer_of.insert(i, layer.id);
        built.targets.insert(i, Target::Layer(layer.id, if top { [origin.0, origin.1] } else { [0.0; 2] }));
        layers.push(layer);
    }
    // Rive multiplies a node's opacity into its children; layer parenting does not.
    for (i, o) in ab.objects.iter().enumerate().skip(1) {
        let i = i as u64;
        if layer_of.contains_key(&i) && ab.children(i).iter().any(|c| layer_of.contains_key(c)) {
            let animated = ab.animations.iter().any(|a| a.keyed.iter().any(|k| k.object == i && k.properties.iter().any(|p| p.key == keys::OPACITY)));
            if animated || (o.float_or(keys::OPACITY, 1.0) - 1.0).abs() > 1e-6 {
                warn(res, format!("“{}”: opacity of a parent doesn't carry into its child layers", o.name()));
            }
        }
    }
    for (i, o) in ab.objects.iter().enumerate().skip(1) {
        let i = i as u64;
        if built.targets.contains_key(&i) || is_content(o.type_key()) {
            continue;
        }
        let n = o.type_name();
        if !n.ends_with("Interpolator") && n != "LayoutComponentStyle" {
            warn(res, format!("{n} not imported"));
        }
    }
    comp.layers = layers;
    for sm in &ab.state_machines {
        warn(res, format!("state machine “{}” not imported (its animations are)", sm.name));
    }
    let base_name = if ab.name.is_empty() { "Artboard".to_string() } else { ab.name.clone() };
    let base = project.add_item(&base_name, Label::Sandstone, Some(folder), ItemKind::Comp(Arc::new(comp)));
    res.bases.push(base);

    for anim in &ab.animations {
        match import_animation(project, ab, base, anim, &built, res) {
            Ok(t) => res.timelines.push((t, base)),
            Err(e) => warn(res, format!("animation “{}”: {e}", anim.name)),
        }
    }
}

fn is_content(t: u32) -> bool {
    matches!(t, ty::RECTANGLE | ty::ELLIPSE | ty::FILL | ty::STROKE | ty::SOLID_COLOR)
}

/// Position as separated X/Y/Z Position (Rive keys x and y independently).
fn separate_position(tr: &mut effectcraft_project::PropGroup, xy: [f64; 2], next: &mut u64) {
    if let Some(p) = tr.get_mut("position") {
        p.value = Value::Vec3([xy[0], xy[1], 0.0]);
    }
    let at = tr.children.iter().position(|c| c.match_id() == "position").map(|i| i + 1).unwrap_or(tr.children.len());
    for (d, (m, name)) in [("positionX", "X Position"), ("positionY", "Y Position"), ("positionZ", "Z Position")].iter().enumerate().rev() {
        *next += 1;
        let mut pr = Property::new(*next, m, name, Value::Scalar(if d < 2 { xy[d] } else { 0.0 })).with_ui(ParamUi::Number);
        pr.three_d_only = d == 2;
        tr.children.insert(at, PropNode::Prop(pr));
    }
}

/// A shape's paths, fills and strokes as shape layer contents.
fn shape_contents(project: &mut Project, ab: &Artboard, shape: u64, layer: &mut effectcraft_project::Layer, built: &mut Built, res: &mut ImportResult) {
    let mut paths = Vec::new();
    let mut paints = Vec::new();
    let lid = layer.id;
    for c in ab.children(shape) {
        let Some(o) = ab.object(c) else { continue };
        let mut ids = Ids(&mut project.next_id);
        match o.type_key() {
            ty::RECTANGLE | ty::ELLIPSE => {
                let (w, h) = (o.float_or(keys::PATH_WIDTH, 0.0), o.float_or(keys::PATH_HEIGHT, 0.0));
                let origin = [o.float_or(keys::PATH_ORIGIN_X, 0.5), o.float_or(keys::PATH_ORIGIN_Y, 0.5)];
                let offset = [o.float_or(keys::X, 0.0), o.float_or(keys::Y, 0.0)];
                let centre = [offset[0] + (0.5 - origin[0]) * w, offset[1] + (0.5 - origin[1]) * h];
                if o.float_or(keys::ROTATION, 0.0).abs() > 1e-9
                    || (o.float_or(keys::SCALE_X, 1.0) - 1.0).abs() > 1e-9
                    || (o.float_or(keys::SCALE_Y, 1.0) - 1.0).abs() > 1e-9
                {
                    push_warn(res, format!("“{}”: rotation and scale of a path inside a shape not imported", o.name()));
                }
                let mut g = if o.type_key() == ty::RECTANGLE {
                    let r = o.float_or(31, 0.0);
                    build::shape_rect(&mut ids, [w, h], centre, r)
                } else {
                    build::shape_ellipse(&mut ids, [w, h], centre)
                };
                if !o.name().is_empty() {
                    g.name = o.name().to_string();
                }
                let (Some(size), Some(position)) = (g.get("size").map(|p| p.uid), g.get("position").map(|p| p.uid)) else { continue };
                built.targets.insert(c, Target::Path { layer: lid, size, position, offset, origin });
                paths.push(g);
            }
            ty::FILL | ty::STROKE => {
                let colour = ab.children(c).into_iter().find(|i| ab.object(*i).is_some_and(|x| x.type_key() == ty::SOLID_COLOR));
                if colour.is_none() {
                    push_warn(res, format!("“{}”: gradient paint imported as grey", o.name()));
                }
                let rgba = colour.and_then(|i| ab.object(i)).and_then(|x| x.raw.color(keys::SOLID_COLOR)).map(argb).unwrap_or([0.455, 0.455, 0.455, 1.0]);
                let visible = o.uint_or(41, 1) != 0;
                let mut g = if o.type_key() == ty::FILL {
                    build::shape_fill(&mut ids, [rgba[0], rgba[1], rgba[2], 1.0])
                } else {
                    build::shape_stroke(&mut ids, [rgba[0], rgba[1], rgba[2], 1.0], o.float_or(STROKE_THICKNESS, 1.0))
                };
                if let Some(op) = g.get_mut("opacity") {
                    op.value = Value::Scalar(rgba[3] * 100.0);
                }
                g.enabled = visible;
                if !o.name().is_empty() {
                    g.name = o.name().to_string();
                }
                if let (Some(i), Some(color)) = (colour, g.get("color").map(|p| p.uid)) {
                    built.targets.insert(i, Target::Color { layer: lid, color });
                }
                paints.push(g);
            }
            _ => {}
        }
    }
    // Paths first; then paints, the last-drawn Rive paint first (on top).
    paints.reverse();
    if let Some(contents) = layer.props.sub_mut("contents") {
        for g in paths.into_iter().chain(paints) {
            contents.children.push(g.into());
        }
    }
}

fn push_warn(res: &mut ImportResult, w: String) {
    if !res.warnings.contains(&w) {
        res.warnings.push(w);
    }
}

fn import_animation(
    project: &mut Project,
    ab: &Artboard,
    base: ItemId,
    anim: &LinearAnimation,
    built: &Built,
    res: &mut ImportResult,
) -> Result<ItemId, RiveError> {
    let fps = anim.fps.clamp(1, 1000);
    let rate = rate_of(fps);
    let duration = rate.snap_nearest(frames_to_tick(anim.duration.max(1) as f64, fps)).max(rate.frame_duration());
    let name = if anim.name.is_empty() { "Animation".to_string() } else { anim.name.clone() };
    let id = linked::new_timeline(project, base, &name, Some(duration)).map_err(|e| RiveError::Malformed(e.to_string()))?;
    if let Some(it) = project.item_mut(id) {
        it.comment = format!(
            "Rive animation: {}{}",
            match anim.looping {
                Loop::OneShot => "one shot",
                Loop::Loop => "loop",
                Loop::PingPong => "ping-pong",
            },
            if (anim.speed - 1.0).abs() > 1e-9 { format!(", speed {}", anim.speed) } else { String::new() }
        );
    }
    let Some(mut comp) = project.comp(id).cloned() else { return Ok(id) };
    comp.frame_rate = rate;
    if let Some((s, e)) = anim.work_area {
        comp.work_area = (frames_to_tick(s as f64, fps), frames_to_tick(e.max(s + 1) as f64, fps).min(duration));
    }
    for ko in &anim.keyed {
        let Some(target) = built.targets.get(&ko.object).copied() else {
            let what = ab.object(ko.object).map(|o| o.type_name()).unwrap_or_else(|| format!("object {}", ko.object));
            push_warn(res, format!("“{}”: animation of {what} not imported", anim.name));
            continue;
        };
        let channel = |key: u32| ko.properties.iter().find(|p| p.key == key).map(|p| p.frames.as_slice());
        match target {
            Target::Layer(lid, add) => {
                let Some(tr) = comp.layer_mut(lid).and_then(|l| l.props.sub_mut("transform")) else { continue };
                let deg = 180.0 / std::f64::consts::PI;
                for (key, m, xf) in [
                    (keys::X, "positionX", (1.0, add[0])),
                    (keys::Y, "positionY", (1.0, add[1])),
                    (keys::ROTATION, "rotation", (deg, 0.0)),
                    (keys::OPACITY, "opacity", (100.0, 0.0)),
                ] {
                    if let (Some(ch), Some(p)) = (channel(key), tr.get_mut(m)) {
                        set_channels(p, &[Some(ch)], &[xf], ab, fps, anim, res);
                    }
                }
                if (channel(keys::SCALE_X).is_some() || channel(keys::SCALE_Y).is_some())
                    && let Some(p) = tr.get_mut("scale")
                {
                    set_channels(p, &[channel(keys::SCALE_X), channel(keys::SCALE_Y)], &[(100.0, 0.0), (100.0, 0.0)], ab, fps, anim, res);
                }
            }
            Target::Path { layer, size, position, offset, origin } => {
                let (cw, chh) = (channel(keys::PATH_WIDTH), channel(keys::PATH_HEIGHT));
                if cw.is_none() && chh.is_none() {
                    push_warn(res, format!("“{}”: path animation other than width/height not imported", anim.name));
                    continue;
                }
                let Some(l) = comp.layer_mut(layer) else { continue };
                if let Some(p) = l.props.find_mut(size) {
                    set_channels(p, &[cw, chh], &[(1.0, 0.0), (1.0, 0.0)], ab, fps, anim, res);
                }
                // Off-centre origins move the centre as the size changes.
                if ((origin[0] - 0.5).abs() > 1e-9 && cw.is_some()) || ((origin[1] - 0.5).abs() > 1e-9 && chh.is_some()) {
                    let sz = l.props.find(size).cloned();
                    if let (Some(sz), Some(p)) = (sz, l.props.find_mut(position)) {
                        let n = anim.duration.max(1);
                        p.keys = (0..=n)
                            .map(|f| {
                                let t = frames_to_tick(f as f64, fps);
                                let s = sz.value_at(t).components();
                                let (w, h) = (s.first().copied().unwrap_or(0.0), s.get(1).copied().unwrap_or(0.0));
                                Keyframe::new(t, Value::Vec2([offset[0] + (0.5 - origin[0]) * w, offset[1] + (0.5 - origin[1]) * h]))
                            })
                            .collect();
                    }
                }
            }
            Target::Color { layer, color } => {
                let Some(frames) = ko.properties.iter().find(|p| p.key == keys::SOLID_COLOR).map(|p| p.frames.as_slice()) else { continue };
                let Some(p) = comp.layer_mut(layer).and_then(|l| l.props.find_mut(color)) else { continue };
                set_color(p, frames, fps);
            }
        }
        for p in &ko.properties {
            let handled = matches!(
                (target, p.key),
                (Target::Layer(..), keys::X | keys::Y | keys::ROTATION | keys::SCALE_X | keys::SCALE_Y | keys::OPACITY)
                    | (Target::Path { .. }, keys::PATH_WIDTH | keys::PATH_HEIGHT)
                    | (Target::Color { .. }, keys::SOLID_COLOR)
            );
            if !handled {
                let prop = crate::property(p.key).map(|(n, _)| n.to_string()).unwrap_or_else(|| format!("property {}", p.key));
                let obj = ab.object(ko.object).map(|o| o.name().to_string()).unwrap_or_default();
                push_warn(res, format!("“{}”: keys on “{obj}” {prop} not imported", anim.name));
            }
        }
    }
    if let Some(c) = project.comp_mut(id) {
        *c = comp;
    }
    Ok(id)
}

/// A Rive channel's value at a (fractional) frame, following its interpolation.
pub fn channel_value(ab: &Artboard, frames: &[KeyFrame], frame: f64) -> f64 {
    let v = |k: &KeyFrame| match k.value {
        KeyValue::Double(d) => d,
        _ => 0.0,
    };
    let Some(first) = frames.first() else { return 0.0 };
    if frame <= first.frame as f64 {
        return v(first);
    }
    for w in frames.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if frame < b.frame as f64 {
            let span = (b.frame as f64 - a.frame as f64).max(1e-9);
            let x = (frame - a.frame as f64) / span;
            let p = match a.interp {
                Interp::Hold => 0.0,
                Interp::Cubic => match a.interpolator.and_then(|i| ab.interpolator(i)) {
                    Some(Interpolator::Cubic { x1, y1, x2, y2 }) => cubic_ease(x1, y1, x2, y2, x),
                    _ => x,
                },
                _ => x,
            };
            return v(a) + (v(b) - v(a)) * p;
        }
    }
    frames.last().map(v).unwrap_or(0.0)
}

/// Progress of a CSS-style cubic timing curve at time fraction `x`.
pub fn cubic_ease(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    let bez = |a: f64, b: f64, t: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t;
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if bez(x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0), mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(y1, y2, 0.5 * (lo + hi))
}

/// Set `p`'s keys from Rive channels, one per dimension (`None`: the dimension keeps its
/// value), each mapped by its `(multiply, add)`.
fn set_channels(
    p: &mut Property,
    channels: &[Option<&[KeyFrame]>],
    xf: &[(f64, f64)],
    ab: &Artboard,
    fps: u64,
    anim: &LinearAnimation,
    res: &mut ImportResult,
) {
    let base = p.value.components();
    let present: Vec<&[KeyFrame]> = channels.iter().flatten().copied().collect();
    let Some(first) = present.first() else { return };
    let frames_of = |ch: &[KeyFrame]| ch.iter().map(|k| (k.frame, k.interp, k.interpolator)).collect::<Vec<_>>();
    let shared = present.iter().all(|ch| frames_of(ch) == frames_of(first));
    let unsupported = present.iter().any(|ch| ch.iter().any(|k| matches!(k.interp, Interp::CubicValue | Interp::Elastic | Interp::Other(_))));
    if unsupported {
        push_warn(res, format!("“{}”: elastic and value-curve easing imported as linear", anim.name));
    }
    let value_at = |dims: &dyn Fn(usize) -> f64| p.value.with_components(&(0..base.len()).map(dims).collect::<Vec<_>>());
    let dim_value = |d: usize, f: f64| match channels.get(d).copied().flatten() {
        Some(ch) => {
            let (m, a) = xf.get(d).copied().unwrap_or((1.0, 0.0));
            channel_value(ab, ch, f) * m + a
        }
        None => base.get(d).copied().unwrap_or(0.0),
    };
    if !shared {
        // Channels keyed at different frames: bake every frame.
        let n = anim.duration.max(1);
        p.keys = (0..=n).map(|f| Keyframe::new(frames_to_tick(f as f64, fps), value_at(&|d| dim_value(d, f as f64)))).collect();
        return;
    }
    let mut out: Vec<Keyframe> =
        first.iter().map(|k| Keyframe::new(frames_to_tick(k.frame as f64, fps), value_at(&|d| dim_value(d, k.frame as f64)))).collect();
    let n = out.len();
    for i in 0..n.saturating_sub(1) {
        let Some(k) = first.get(i) else { break };
        let (a, b) = (out[i].time.seconds(), out[i + 1].time.seconds());
        let dur = (b - a).max(1e-9);
        match k.interp {
            Interp::Hold => {
                out[i].out_interp = EcInterp::Hold;
                out[i + 1].in_interp = EcInterp::Hold;
            }
            Interp::Cubic => {
                let Some(Interpolator::Cubic { x1, y1, x2, y2 }) = k.interpolator.and_then(|j| ab.interpolator(j)) else { continue };
                if (x1 - y1).abs() < 1e-9 && (x2 - y2).abs() < 1e-9 {
                    continue;
                }
                let (va, vb) = (out[i].value.components(), out[i + 1].value.components());
                let infl_o = x1.clamp(0.001, 1.0);
                let infl_i = (1.0 - x2).clamp(0.001, 1.0);
                let dims = va.len().max(1);
                out[i].out_interp = EcInterp::Bezier;
                out[i].out_ease = (0..dims)
                    .map(|d| Ease { speed: y1 * (vb.get(d).unwrap_or(&0.0) - va.get(d).unwrap_or(&0.0)) / (infl_o * dur), influence: infl_o })
                    .collect();
                out[i + 1].in_interp = EcInterp::Bezier;
                out[i + 1].in_ease = (0..dims)
                    .map(|d| Ease { speed: (1.0 - y2) * (vb.get(d).unwrap_or(&0.0) - va.get(d).unwrap_or(&0.0)) / (infl_i * dur), influence: infl_i })
                    .collect();
            }
            _ => {}
        }
    }
    if let Some(k) = out.first() {
        p.value = k.value.clone();
    }
    p.keys = out;
}

/// Colour keys (`KeyFrameColor`): channel-wise, like Rive.
fn set_color(p: &mut Property, frames: &[KeyFrame], fps: u64) {
    let mut out: Vec<Keyframe> = frames
        .iter()
        .filter_map(|k| match k.value {
            KeyValue::Color(c) => Some(Keyframe::new(frames_to_tick(k.frame as f64, fps), Value::Color(argb(c)))),
            _ => None,
        })
        .collect();
    let n = out.len().min(frames.len());
    for i in 0..n.saturating_sub(1) {
        if frames[i].interp == Interp::Hold {
            out[i].out_interp = EcInterp::Hold;
            out[i + 1].in_interp = EcInterp::Hold;
        }
    }
    if let Some(k) = out.first() {
        p.value = k.value.clone();
    }
    p.keys = out;
}
