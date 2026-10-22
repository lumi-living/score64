//! JSON Schemas for the validated newtypes, with the ranges and patterns their `TryFrom` checks. Derived types get
//! theirs from `#[derive(JsonSchema)]`.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema as JsonSchemaDoc, SchemaGenerator, json_schema};

use crate::provenance::{Confidence, ContentHash, DecisionId};
use crate::score::{FramesPerRow, Pitch, SCHEMA_VERSION, Schema, SubtuneName, Voice};

macro_rules! schema {
    ($t:ty, $name:literal, $body:tt) => {
        impl JsonSchema for $t {
            fn schema_name() -> Cow<'static, str> {
                $name.into()
            }
            fn json_schema(_: &mut SchemaGenerator) -> JsonSchemaDoc {
                json_schema!($body)
            }
        }
    };
}

schema!(Schema, "Schema", { "description": "Score layout version.", "const": SCHEMA_VERSION });
schema!(Voice, "Voice", { "description": "SID voice.", "type": "integer", "minimum": 1, "maximum": 3 });
schema!(Pitch, "Pitch", {
    "description": "GoatTracker pitch from C-0 to G#7.",
    "type": "string",
    "pattern": "^((C-|C#|D-|D#|E-|F-|F#|G-|G#|A-|A#|B-)[0-6]|(C-|C#|D-|D#|E-|F-|F#|G-|G#)7)$"
});
schema!(ContentHash, "ContentHash", {
    "description": "blake3 hash, 64 hex digits.",
    "type": "string",
    "pattern": "^[0-9a-fA-F]{64}$"
});
schema!(Confidence, "Confidence", { "type": "number", "minimum": 0, "maximum": 1 });
schema!(DecisionId, "DecisionId", {
    "type": "string",
    "pattern": "^[a-z0-9][a-z0-9_-]{0,63}$"
});
schema!(SubtuneName, "SubtuneName", {
    "type": "string",
    "pattern": "^[a-z0-9][a-z0-9_-]{0,63}$"
});
schema!(FramesPerRow, "FramesPerRow", {
    "description": "GoatTracker tempo; below 3 is funktempo, which the score does not model.",
    "type": "integer",
    "minimum": 3,
    "maximum": 127
});
