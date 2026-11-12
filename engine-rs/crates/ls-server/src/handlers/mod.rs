// handlers split by domain (2026-11-12 batch A); fns re-exported here for centralized router registration.
pub(crate) mod alerts;
pub(crate) mod cameras;
pub(crate) mod config_io;
pub(crate) mod events;
pub(crate) mod export;
pub(crate) mod folder_images;
pub(crate) mod misc_status;
pub(crate) mod ops;
pub(crate) mod pairing;
pub(crate) mod stats;
pub(crate) mod zones_rules;

pub(crate) use alerts::*;
pub(crate) use cameras::*;
pub(crate) use config_io::*;
pub(crate) use events::*;
pub(crate) use export::*;
pub(crate) use folder_images::*;
pub(crate) use misc_status::*;
pub(crate) use ops::*;
pub(crate) use pairing::*;
pub(crate) use stats::*;
pub(crate) use zones_rules::*;
