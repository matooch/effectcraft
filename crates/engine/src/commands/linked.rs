//! Linked timelines (`effectcraft_project::linked`): compositions that animate a base
//! composition's layers, as Rive animations animate an artboard.

use effectcraft_project::linked;
use effectcraft_time::Tick;
use serde_json::{Value, json};

use super::{CommandSpec, b_p, bad, comp_id, f_p, has_comp, str_p};
use crate::{EngineError, Result, Session, cmd, query};

fn new_timeline(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "comp.newLinkedTimeline";
    let base = comp_id(s, p)?;
    let base = linked::base_of(&s.project, base).unwrap_or(base);
    let duration = match f_p(p, "duration") {
        Some(d) if d.is_finite() && d > 0.0 => Some(Tick::from_seconds_f64(d)),
        Some(_) => return Err(bad(c, "`duration` must be a positive number of seconds")),
        None => None,
    };
    let base_name = s.project.item(base).map(|i| i.name.clone()).unwrap_or_default();
    let name = str_p(p, "name").map(str::to_string).unwrap_or_else(|| format!("{base_name} Timeline"));
    let id = s.edit("New Linked Timeline", None, |proj, st| {
        let mut unique = name.clone();
        let mut k = 2;
        while proj.items.values().any(|i| i.name == unique) {
            unique = format!("{name} {k}");
            k += 1;
        }
        let id = linked::new_timeline(proj, base, &unique, duration).map_err(|e| EngineError::Other(e.to_string()))?;
        st.project_selection = vec![id];
        Ok(id)
    })?;
    if b_p(p, "open") != Some(false) {
        s.open_comp(id);
    }
    Ok(json!({"comp": id.0, "base": base.0}))
}

fn unlink(s: &mut Session, p: &Value) -> Result<Value> {
    let cid = comp_id(s, p)?;
    if s.project.comp(cid).and_then(|c| c.timeline_of).is_none() {
        return Err(bad("comp.unlinkTimeline", "the composition is not a linked timeline"));
    }
    s.edit("Unlink Timeline", None, |proj, _| {
        linked::unlink(proj, cid);
        Ok(())
    })?;
    Ok(json!({"comp": cid.0}))
}

/// The base of a composition (itself when it is not a timeline) and the base's timelines.
fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let cid = comp_id(s, p)?;
    let base = linked::base_of(&s.project, cid).unwrap_or(cid);
    let item = |id: effectcraft_project::ItemId| json!({"id": id.0, "name": s.project.item(id).map(|i| i.name.as_str()).unwrap_or_default()});
    let timelines: Vec<Value> = linked::timelines_of(&s.project, base).into_iter().map(item).collect();
    Ok(json!({"base": item(base), "timelines": timelines}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "comp.newLinkedTimeline",
            "New Linked Timeline",
            [],
            None,
            "{comp? (the base; a timeline means its base), name?, duration? (s, default the base's), open?: bool}",
            has_comp,
            new_timeline
        ),
        cmd!("comp.unlinkTimeline", "Unlink Timeline", [], None, "{comp?}", has_comp, unlink),
        query!("comp.linkedTimelines", "Linked Timelines", "{comp?} → {base: {id, name}, timelines: [{id, name}]}", list),
    ]
}
