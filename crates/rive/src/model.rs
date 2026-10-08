//! The structure of a `.riv` object stream: artboards, their object lists and their linear
//! animations.

use std::collections::BTreeMap;

use crate::{RawObject, RivFile, RiveError, in_artboard, type_label};

/// Property keys of the object model used here.
pub mod keys {
    pub const NAME: u32 = 4;
    pub const PARENT_ID: u32 = 5;
    /// Artboard / layout width and height.
    pub const ARTBOARD_WIDTH: u32 = 7;
    pub const ARTBOARD_HEIGHT: u32 = 8;
    pub const ARTBOARD_ORIGIN_X: u32 = 11;
    pub const ARTBOARD_ORIGIN_Y: u32 = 12;
    pub const X: u32 = 13;
    pub const Y: u32 = 14;
    /// Radians.
    pub const ROTATION: u32 = 15;
    pub const SCALE_X: u32 = 16;
    pub const SCALE_Y: u32 = 17;
    pub const OPACITY: u32 = 18;
    /// Parametric path (rectangle, ellipse…) width and height.
    pub const PATH_WIDTH: u32 = 20;
    pub const PATH_HEIGHT: u32 = 21;
    pub const SOLID_COLOR: u32 = 37;
    pub const KEYED_OBJECT_ID: u32 = 51;
    pub const KEYED_PROPERTY_KEY: u32 = 53;
    pub const ANIMATION_NAME: u32 = 55;
    pub const FPS: u32 = 56;
    pub const DURATION: u32 = 57;
    pub const SPEED: u32 = 58;
    pub const LOOP: u32 = 59;
    pub const WORK_START: u32 = 60;
    pub const WORK_END: u32 = 61;
    pub const ENABLE_WORK_AREA: u32 = 62;
    pub const CUBIC_X1: u32 = 63;
    pub const CUBIC_Y1: u32 = 64;
    pub const CUBIC_X2: u32 = 65;
    pub const CUBIC_Y2: u32 = 66;
    pub const FRAME: u32 = 67;
    pub const INTERPOLATION_TYPE: u32 = 68;
    pub const INTERPOLATOR_ID: u32 = 69;
    pub const VALUE_DOUBLE: u32 = 70;
    pub const VALUE_COLOR: u32 = 88;
    pub const VALUE_ID: u32 = 122;
    pub const VALUE_BOOL: u32 = 181;
    pub const VALUE_STRING: u32 = 280;
    pub const VALUE_UINT: u32 = 631;
    pub const VALUE_INT: u32 = 1068;
    /// Parametric path origin (0..1 of its size; default 0.5).
    pub const PATH_ORIGIN_X: u32 = 123;
    pub const PATH_ORIGIN_Y: u32 = 124;
    pub const STATE_MACHINE_NAME: u32 = 138;
}

mod types {
    pub const ARTBOARD: u32 = 1;
    pub const BACKBOARD: u32 = 23;
    pub const KEYED_OBJECT: u32 = 25;
    pub const KEYED_PROPERTY: u32 = 26;
    pub const CUBIC_EASE: u32 = 28;
    pub const KEY_FRAME_DOUBLE: u32 = 30;
    pub const LINEAR_ANIMATION: u32 = 31;
    pub const KEY_FRAME_COLOR: u32 = 37;
    pub const KEY_FRAME_ID: u32 = 50;
    pub const STATE_MACHINE: u32 = 53;
    pub const KEY_FRAME_BOOL: u32 = 84;
    pub const CUBIC_VALUE: u32 = 138;
    pub const KEY_FRAME_STRING: u32 = 142;
    pub const KEY_FRAME_CALLBACK: u32 = 171;
    pub const ELASTIC: u32 = 174;
    pub const KEY_FRAME_UINT: u32 = 450;
    pub const KEY_FRAME_INT: u32 = 1067;
}

/// One entry of an artboard's object list.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub raw: RawObject,
}

impl Object {
    pub fn type_key(&self) -> u32 {
        self.raw.type_key
    }
    pub fn type_name(&self) -> String {
        type_label(self.raw.type_key)
    }
    pub fn name(&self) -> &str {
        self.raw.string(keys::NAME).unwrap_or_default()
    }
    /// Index of the parent in the artboard's object list (absent means the artboard, 0).
    pub fn parent(&self) -> u64 {
        self.raw.uint(keys::PARENT_ID).unwrap_or(0)
    }
    /// A float property, or `default` when the file leaves it at its default (omitted).
    pub fn float_or(&self, key: u32, default: f64) -> f64 {
        self.raw.float(key).map(f64::from).unwrap_or(default)
    }
    pub fn uint_or(&self, key: u32, default: u64) -> u64 {
        self.raw.uint(key).unwrap_or(default)
    }
}

/// How the segment after a keyframe interpolates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Hold,
    Linear,
    /// Cubic timing curve (a [`Interpolator::Cubic`] gives the curve).
    Cubic,
    /// Cubic curve over the values (`CubicValueInterpolator`).
    CubicValue,
    Elastic,
    Other(u64),
}

impl Interp {
    fn from_u64(v: u64) -> Interp {
        match v {
            0 => Interp::Hold,
            1 => Interp::Linear,
            2 => Interp::Cubic,
            3 => Interp::CubicValue,
            4 => Interp::Elastic,
            n => Interp::Other(n),
        }
    }
}

/// A keyframe interpolator of the artboard's object list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Interpolator {
    /// `CubicEaseInterpolator`: a timing curve through (0,0), (x1,y1), (x2,y2), (1,1).
    Cubic {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
    /// `CubicValueInterpolator`: the same control points over the value.
    CubicValue {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    },
    /// `ElasticInterpolator`.
    Elastic {
        easing: u64,
        amplitude: f64,
        period: f64,
    },
    Unsupported(u32),
}

/// A keyframe's value.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyValue {
    Double(f64),
    /// `0xAARRGGBB`.
    Color(u32),
    Bool(bool),
    Uint(u64),
    Id(u64),
    String(String),
    /// A callback keyframe (fires, carries no value).
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyFrame {
    /// Frame number at the animation's fps.
    pub frame: u64,
    pub value: KeyValue,
    /// Interpolation of the segment that starts at this key.
    pub interp: Interp,
    /// Index of the interpolator in the artboard's object list.
    pub interpolator: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyedProperty {
    pub key: u32,
    pub frames: Vec<KeyFrame>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyedObject {
    /// Index of the animated object in the artboard's object list.
    pub object: u64,
    pub properties: Vec<KeyedProperty>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Loop {
    #[default]
    OneShot,
    Loop,
    PingPong,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinearAnimation {
    pub name: String,
    pub fps: u64,
    /// Frames.
    pub duration: u64,
    pub speed: f64,
    pub looping: Loop,
    /// Work area in frames, when enabled.
    pub work_area: Option<(u64, u64)>,
    pub keyed: Vec<KeyedObject>,
}

/// A state machine (not modelled further: its name and how many objects it holds).
#[derive(Clone, Debug, PartialEq)]
pub struct StateMachineInfo {
    pub name: String,
    pub objects: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Artboard {
    pub name: String,
    pub width: f64,
    pub height: f64,
    /// The object list: the artboard itself at 0, then its components and interpolators in file
    /// order. `parentId`, `objectId` and `interpolatorId` index it.
    pub objects: Vec<Object>,
    pub animations: Vec<LinearAnimation>,
    pub state_machines: Vec<StateMachineInfo>,
}

impl Artboard {
    pub fn object(&self, index: u64) -> Option<&Object> {
        self.objects.get(usize::try_from(index).ok()?)
    }
    /// Indices of the objects whose parent is `index` (the artboard itself has no parent).
    pub fn children(&self, index: u64) -> Vec<u64> {
        self.objects.iter().enumerate().skip(1).filter(|(_, o)| o.parent() == index).map(|(i, _)| i as u64).collect()
    }
    /// The interpolator at an object-list index.
    pub fn interpolator(&self, index: u64) -> Option<Interpolator> {
        let o = self.object(index)?;
        let cubic =
            |o: &Object| (o.float_or(keys::CUBIC_X1, 0.42), o.float_or(keys::CUBIC_Y1, 0.0), o.float_or(keys::CUBIC_X2, 0.58), o.float_or(keys::CUBIC_Y2, 1.0));
        Some(match o.type_key() {
            types::CUBIC_EASE => {
                let (x1, y1, x2, y2) = cubic(o);
                Interpolator::Cubic { x1, y1, x2, y2 }
            }
            types::CUBIC_VALUE => {
                let (x1, y1, x2, y2) = cubic(o);
                Interpolator::CubicValue { x1, y1, x2, y2 }
            }
            types::ELASTIC => Interpolator::Elastic { easing: o.uint_or(405, 0), amplitude: o.float_or(406, 1.0), period: o.float_or(407, 1.0) },
            t if crate::in_artboard(t) && crate::type_name(t).is_some_and(|n| n.ends_with("Interpolator")) => Interpolator::Unsupported(t),
            _ => return None,
        })
    }
}

/// A `.riv` file's artboards.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub major: u64,
    pub minor: u64,
    pub artboards: Vec<Artboard>,
    /// What was read but not modelled (state machine parts, assets, view models…), by type
    /// name, and objects that appeared out of context.
    pub skipped: BTreeMap<String, usize>,
}

impl Document {
    /// Read a `.riv` file's bytes.
    pub fn read(bytes: &[u8]) -> Result<Document, RiveError> {
        Ok(Document::from_file(&RivFile::read(bytes)?))
    }

    /// Structure a flat object stream.
    pub fn from_file(f: &RivFile) -> Document {
        let mut doc = Document { major: f.major, minor: f.minor, artboards: Vec::new(), skipped: BTreeMap::new() };
        // What the next non-artboard object belongs to.
        #[derive(PartialEq)]
        enum Ctx {
            None,
            Animation,
            StateMachine,
        }
        let mut ctx = Ctx::None;
        let skip = |doc: &mut Document, what: String| *doc.skipped.entry(what).or_insert(0) += 1;
        for o in &f.objects {
            let t = o.type_key;
            // File-level editor settings; nothing to model.
            if t == types::BACKBOARD {
                continue;
            }
            if t == types::ARTBOARD {
                doc.artboards.push(Artboard {
                    name: o.string(keys::NAME).unwrap_or_default().to_string(),
                    width: o.float(keys::ARTBOARD_WIDTH).map(f64::from).unwrap_or(0.0),
                    height: o.float(keys::ARTBOARD_HEIGHT).map(f64::from).unwrap_or(0.0),
                    objects: vec![Object { raw: o.clone() }],
                    animations: Vec::new(),
                    state_machines: Vec::new(),
                });
                ctx = Ctx::None;
                continue;
            }
            let Some(ab) = doc.artboards.last_mut() else {
                skip(&mut doc, type_label(t));
                continue;
            };
            if in_artboard(t) {
                ab.objects.push(Object { raw: o.clone() });
                continue;
            }
            match t {
                types::LINEAR_ANIMATION => {
                    let work = (o.uint(keys::ENABLE_WORK_AREA).unwrap_or(0) != 0)
                        .then(|| (o.uint(keys::WORK_START).unwrap_or(0), o.uint(keys::WORK_END).unwrap_or(0)));
                    ab.animations.push(LinearAnimation {
                        name: o.string(keys::ANIMATION_NAME).unwrap_or_default().to_string(),
                        fps: o.uint(keys::FPS).unwrap_or(60),
                        duration: o.uint(keys::DURATION).unwrap_or(60),
                        speed: o.float(keys::SPEED).map(f64::from).unwrap_or(1.0),
                        looping: match o.uint(keys::LOOP).unwrap_or(0) {
                            1 => Loop::Loop,
                            2 => Loop::PingPong,
                            _ => Loop::OneShot,
                        },
                        work_area: work,
                        keyed: Vec::new(),
                    });
                    ctx = Ctx::Animation;
                }
                types::KEYED_OBJECT if ctx == Ctx::Animation => {
                    if let Some(a) = ab.animations.last_mut() {
                        a.keyed.push(KeyedObject { object: o.uint(keys::KEYED_OBJECT_ID).unwrap_or(0), properties: Vec::new() });
                    }
                }
                types::KEYED_PROPERTY if ctx == Ctx::Animation => match ab.animations.last_mut().and_then(|a| a.keyed.last_mut()) {
                    Some(ko) => ko.properties.push(KeyedProperty { key: o.uint(keys::KEYED_PROPERTY_KEY).unwrap_or(0) as u32, frames: Vec::new() }),
                    None => skip(&mut doc, "KeyedProperty (no keyed object)".into()),
                },
                _ if ctx == Ctx::Animation && key_frame_value(o).is_some() => {
                    let kf = KeyFrame {
                        frame: o.uint(keys::FRAME).unwrap_or(0),
                        value: key_frame_value(o).unwrap_or(KeyValue::None),
                        interp: Interp::from_u64(o.uint(keys::INTERPOLATION_TYPE).unwrap_or(0)),
                        interpolator: o.uint(keys::INTERPOLATOR_ID),
                    };
                    match ab.animations.last_mut().and_then(|a| a.keyed.last_mut()).and_then(|k| k.properties.last_mut()) {
                        Some(kp) => kp.frames.push(kf),
                        None => skip(&mut doc, "KeyFrame (no keyed property)".into()),
                    }
                }
                types::STATE_MACHINE => {
                    ab.state_machines.push(StateMachineInfo { name: o.string(keys::STATE_MACHINE_NAME).unwrap_or_default().to_string(), objects: 0 });
                    ctx = Ctx::StateMachine;
                }
                _ if ctx == Ctx::StateMachine => {
                    if let Some(sm) = ab.state_machines.last_mut() {
                        sm.objects += 1;
                    }
                }
                _ => skip(&mut doc, type_label(t)),
            }
        }
        doc
    }
}

/// The value of a keyframe object (`None` when the object is not a keyframe).
fn key_frame_value(o: &RawObject) -> Option<KeyValue> {
    Some(match o.type_key {
        types::KEY_FRAME_DOUBLE => KeyValue::Double(o.float(keys::VALUE_DOUBLE).map(f64::from).unwrap_or(0.0)),
        types::KEY_FRAME_COLOR => KeyValue::Color(o.color(keys::VALUE_COLOR).unwrap_or(0)),
        types::KEY_FRAME_ID => KeyValue::Id(o.uint(keys::VALUE_ID).unwrap_or(0)),
        types::KEY_FRAME_BOOL => KeyValue::Bool(o.uint(keys::VALUE_BOOL).unwrap_or(0) != 0),
        types::KEY_FRAME_STRING => KeyValue::String(o.string(keys::VALUE_STRING).unwrap_or_default().to_string()),
        types::KEY_FRAME_UINT => KeyValue::Uint(o.uint(keys::VALUE_UINT).unwrap_or(0)),
        types::KEY_FRAME_INT => KeyValue::Uint(o.uint(keys::VALUE_INT).unwrap_or(0)),
        types::KEY_FRAME_CALLBACK => KeyValue::None,
        _ => return None,
    })
}
