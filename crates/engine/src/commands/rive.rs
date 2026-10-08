//! File ▸ Import ▸ Rive… (`effectcraft-rive`): each artboard as a base composition, each of its
//! animations as a linked timeline.

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, str_p};
use crate::{EngineError, Result, Session, cmd};

fn import_rive(s: &mut Session, p: &Value) -> Result<Value> {
    let path = str_p(p, "path").ok_or_else(|| bad("file.importRive", "missing `path`"))?.to_string();
    let bytes = s.services.read_file(&path).map_err(|e| EngineError::Other(format!("cannot read {path}: {e}")))?;
    let stem = std::path::Path::new(&path).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Rive".into());
    let res = s.edit("Import Rive", None, |proj, st| {
        let r = effectcraft_rive::import(proj, &bytes, &stem).map_err(|e| EngineError::Other(e.to_string()))?;
        st.project_selection = r.bases.clone();
        Ok(r)
    })?;
    if let Some(first) = res.bases.first() {
        s.open_comp(*first);
    }
    let n = res.warnings.len();
    s.toast(if n == 0 { format!("Imported {path}") } else { format!("Imported {path} ({n} warning{})", if n == 1 { "" } else { "s" }) });
    Ok(json!({
        "folder": res.folder.map(|f| f.0),
        "bases": res.bases.iter().map(|b| b.0).collect::<Vec<_>>(),
        "timelines": res.timelines.iter().map(|(t, b)| json!({"comp": t.0, "base": b.0})).collect::<Vec<_>>(),
        "warnings": res.warnings,
    }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!("file.importRive", "Rive...", ["File", "Import"], None, "{path (.riv)}", always, import_rive)]
}
