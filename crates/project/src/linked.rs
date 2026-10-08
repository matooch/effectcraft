//! Linked timelines: compositions that animate another composition's layers.
//!
//! A *linked timeline* is a composition whose [`Comp::timeline_of`] names a *base* composition.
//! The base owns the layers and everything static about them (sources, shape contents, effects,
//! masks, switches, timing, parenting, unanimated values); each of its timelines owns only its
//! own animation, the keyframes of those layers' properties. One base can have any number of
//! timelines. This is how a Rive file is organised (an artboard and its linear animations), and
//! it suits UI states, game sprite actions or alternate takes that must stay structurally
//! identical.
//!
//! A timeline's `layers` are kept *materialised*: a full copy of the base's layers carrying the
//! timeline's keyframes, with the same layer ids and property uids. Rendering, nesting, the
//! timeline panel and every command therefore work on a timeline as on any composition.
//! [`sync_project`] (run after every edit, like the Essential Graphics sync) keeps the copies
//! consistent:
//!
//! 1. **Up:** a timeline that an edit changed pushes everything that is not keyframes to its
//!    base: added, removed, reordered or renamed layers, switches, timing, parenting, added or
//!    removed effects and masks, expressions and unanimated values.
//! 2. **Down:** every timeline's layers are rebuilt from its base, keeping the timeline's own
//!    keyframes (matched by layer id and property uid). A property the timeline does not animate
//!    shows the base's value at time 0; the base's own keyframes do not carry into timelines
//!    (like a Rive artboard, the base is the rest pose).
//!
//! The base's frame size and pixel aspect follow into its timelines (a size changed in a
//! timeline goes up). Frame rate, duration, work area, markers and background are each
//! timeline's own.
//!
//! Ids: layer ids are unique within a composition, and the same id in a base and its timelines
//! is the same layer.

use std::collections::{BTreeMap, BTreeSet};

use effectcraft_time::Tick;

use crate::{Comp, ItemId, ItemKind, Layer, LayerSource, Project, ProjectError, props::Property};

/// The base of `comp` when `comp` is a linked timeline with a valid base (an existing
/// composition that is not itself a timeline).
pub fn base_of(project: &Project, comp: ItemId) -> Option<ItemId> {
    let base = project.comp(comp)?.timeline_of?;
    (base != comp && project.comp(base).is_some_and(|b| b.timeline_of.is_none())).then_some(base)
}

/// The linked timelines of `base`, in item order.
pub fn timelines_of(project: &Project, base: ItemId) -> Vec<ItemId> {
    project.comps().filter(|(id, c)| **id != base && c.timeline_of == Some(base)).map(|(id, _)| *id).collect()
}

/// A layer as a timeline shows it without its own animation: every keyframed property frozen at
/// its value at time 0.
pub fn static_layer(layer: &Layer) -> Layer {
    let mut l = layer.clone();
    l.props.walk_mut(&mut |p| {
        if !p.keys.is_empty() {
            p.value = p.value_at(Tick::ZERO);
            p.keys.clear();
        }
    });
    l
}

/// A new linked timeline of `base` named `name`, with the base's settings and its layers
/// unanimated. When `base` is itself a timeline, the new one links to that timeline's base.
/// `duration`: the timeline's length (default the base's).
pub fn new_timeline(project: &mut Project, base: ItemId, name: &str, duration: Option<Tick>) -> Result<ItemId, ProjectError> {
    let base = base_of(project, base).unwrap_or(base);
    let b = project.comp(base).ok_or(ProjectError::NotComp(base))?;
    if b.timeline_of.is_some() {
        return Err(ProjectError::Invalid("the composition's base is missing".into()));
    }
    let duration = duration.filter(|d| *d > Tick::ZERO).unwrap_or(b.duration);
    let mut c = Comp { display_start: b.display_start, guides: b.guides.clone(), ..b.nested_like(b.width, b.height, duration) };
    c.layers = b.layers.iter().map(static_layer).collect();
    c.timeline_of = Some(base);
    let (label, parent) = project.item(base).map(|i| (i.label, i.parent)).ok_or(ProjectError::NoItem(base))?;
    Ok(project.add_item(name, label, parent, ItemKind::Comp(c.into())))
}

/// Detach a linked timeline from its base: it keeps its layers and keyframes and becomes an
/// ordinary composition. Returns whether it was linked.
pub fn unlink(project: &mut Project, comp: ItemId) -> bool {
    match project.comp_mut(comp) {
        Some(c) if c.timeline_of.is_some() => {
            c.timeline_of = None;
            true
        }
        _ => false,
    }
}

/// Keep linked timelines and their bases consistent after an edit (`before` → `after`): push
/// what an edit changed in a timeline (other than keyframes) to its base, then rebuild every
/// timeline's layers from its base, keeping its keyframes. Timelines whose base is gone (or is
/// itself a timeline) become ordinary compositions.
///
/// Fails, leaving `after` partly synced (the caller discards it), when the edit would make a
/// timeline nest itself, e.g. a base holding one of its own timelines as a precomp layer.
pub fn sync_project(before: &Project, after: &mut Project) -> Result<(), ProjectError> {
    let linked: Vec<(ItemId, Option<ItemId>)> = after.comps().filter(|(_, c)| c.timeline_of.is_some()).map(|(id, _)| (*id, base_of(after, *id))).collect();
    if linked.is_empty() {
        return Ok(());
    }
    // Broken links: keep the layers, drop the link.
    for (id, base) in &linked {
        if base.is_none() {
            unlink(after, *id);
        }
    }
    let linked: Vec<(ItemId, ItemId)> = linked.into_iter().filter_map(|(id, b)| Some((id, b?))).collect();

    // 1. Up: timelines this edit changed push their static changes to the base.
    for (id, base) in &linked {
        let (Some(ta), Some(tb)) = (after.comp_arc(*id), before.comp(*id)) else { continue };
        // A comp that just became a timeline of this base has nothing to push.
        if tb.timeline_of != Some(*base) || (tb.layers == ta.layers && tb.width == ta.width && tb.height == ta.height && tb.pixel_aspect == ta.pixel_aspect) {
            continue;
        }
        let Some(b) = after.comp(*base) else { continue };
        let pushed = push_up(tb, &ta, b);
        if pushed != *b
            && let Some(bm) = after.comp_mut(*base)
        {
            *bm = pushed;
        }
    }

    // 2. Down: rebuild every timeline from its (possibly just updated) base.
    for (id, base) in &linked {
        let (Some(t), Some(b)) = (after.comp(*id), after.comp(*base)) else { continue };
        let layers: Vec<Layer> = b.layers.iter().map(|bl| compose(bl, t.layer(bl.id))).collect();
        let (w, h, pa) = (b.width, b.height, b.pixel_aspect);
        if (t.layers != layers || t.width != w || t.height != h || t.pixel_aspect != pa)
            && let Some(tm) = after.comp_mut(*id)
        {
            tm.layers = layers;
            tm.width = w;
            tm.height = h;
            tm.pixel_aspect = pa;
        }
    }

    // A base nesting one of its own timelines (directly or through other comps) would make that
    // timeline, which mirrors the base's layers, nest itself.
    for (id, _) in &linked {
        let Some(t) = after.comp(*id) else { continue };
        let nests_itself = t.layers.iter().any(|l| matches!(l.source, LayerSource::Comp { item } if after.comp_contains(item, *id)));
        if nests_itself {
            let name = after.item(*id).map(|i| i.name.clone()).unwrap_or_default();
            return Err(ProjectError::Invalid(format!("the linked timeline \u{201c}{name}\u{201d} would contain itself")));
        }
    }
    Ok(())
}

/// The base after timeline `ta` (which was `tb` before the edit) pushed its changes to `base`.
fn push_up(tb: &Comp, ta: &Comp, base: &Comp) -> Comp {
    let mut out = base.clone();
    if (tb.width, tb.height, tb.pixel_aspect) != (ta.width, ta.height, ta.pixel_aspect) {
        out.width = ta.width;
        out.height = ta.height;
        out.pixel_aspect = ta.pixel_aspect;
    }
    let before_ids: BTreeSet<_> = tb.layers.iter().map(|l| l.id).collect();
    let after_ids: BTreeSet<_> = ta.layers.iter().map(|l| l.id).collect();
    // The timeline's layer order, each layer the base's unless the edit changed it here.
    let mut layers: Vec<Layer> = ta
        .layers
        .iter()
        .map(|la| {
            let lb = tb.layer(la.id);
            let lbase = base.layer(la.id);
            match (lb, lbase) {
                (Some(lb), Some(lbase)) if lb == la => lbase.clone(),
                _ => push_layer(lb, la, lbase),
            }
        })
        .collect();
    // Base layers the timeline never had (added to the base by the same edit) stay, at their
    // place in the base; layers deleted in the timeline go.
    for (i, l) in base.layers.iter().enumerate() {
        if !after_ids.contains(&l.id) && !before_ids.contains(&l.id) {
            let at = i.min(layers.len());
            layers.insert(at, l.clone());
        }
    }
    out.layers = layers;
    out
}

/// The base's layer after the timeline changed `before` into `after` (`base`: the base's layer,
/// `None` for a layer added in the timeline).
fn push_layer(before: Option<&Layer>, after: &Layer, base: Option<&Layer>) -> Layer {
    let mut l = after.clone();
    l.props.walk_mut(&mut |pa| {
        let pb = before.and_then(|b| b.props.find(pa.uid));
        let pbase = base.and_then(|b| b.props.find(pa.uid));
        *pa = push_prop(pa, pb, pbase);
    });
    l
}

/// The base's property after the timeline changed `pb` into `pa`: everything but the value
/// channel comes from the timeline. The value goes up only when the timeline set an unanimated
/// value; keyframes never go up.
fn push_prop(pa: &Property, pb: Option<&Property>, pbase: Option<&Property>) -> Property {
    let animated = !pa.keys.is_empty() || pb.is_some_and(|p| !p.keys.is_empty());
    let value_set = !animated && pb.is_none_or(|pb| pb.value != pa.value);
    let mut np = pa.clone();
    match pbase {
        Some(x) => {
            np.keys = x.keys.clone();
            np.value = if value_set { pa.value.clone() } else { x.value.clone() };
        }
        None => {
            np.value = pa.value_at(Tick::ZERO);
            np.keys.clear();
        }
    }
    np
}

/// A timeline's layer: the base's layer unanimated, with the timeline's own keyframes (`own`:
/// the timeline's current copy of the layer).
fn compose(base: &Layer, own: Option<&Layer>) -> Layer {
    let mut l = static_layer(base);
    let Some(own) = own else { return l };
    let mut keyed: BTreeMap<crate::props::Uid, &Property> = BTreeMap::new();
    own.props.walk("", &mut |_, p| {
        if !p.keys.is_empty() {
            keyed.insert(p.uid, p);
        }
    });
    if keyed.is_empty() {
        return l;
    }
    l.props.walk_mut(&mut |p| {
        if let Some(o) = keyed.get(&p.uid) {
            p.keys = o.keys.clone();
            p.value = o.value.clone();
        }
    });
    l
}
