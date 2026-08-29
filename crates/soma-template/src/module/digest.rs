//! Canonical module encoding whose SHA-256 is the module digest bound into a lock.

use sha2::{Digest as _, Sha256};

use super::{HealthProbe, ModuleSpec};
use crate::{schema::Command, wire::Writer};

const MAGIC: &[u8; 8] = b"SOMAMODL";
const ENCODING_VERSION: u16 = 1;

pub(super) fn digest(spec: &ModuleSpec) -> [u8; 32] {
    let mut writer = Writer::with_capacity(512);
    writer.put_bytes(MAGIC);
    writer.put_u16(ENCODING_VERSION);
    writer.put_u8(spec.identity().kind().code());
    writer.put_string(spec.identity().name());
    writer.put_u32(spec.identity().version());
    writer.put_u16(spec.schema_version());
    writer.put_count(spec.requires().len());
    for reference in spec.requires() {
        writer.put_string(&reference.to_string());
    }
    writer.put_strings(spec.exclusive_fields());
    writer.put_count(spec.owned_paths().len());
    for path in spec.owned_paths() {
        writer.put_string(path.as_str());
    }
    writer.put_count(spec.executables().len());
    for path in spec.executables() {
        writer.put_string(path.as_str());
    }
    writer.put_count(spec.required_environment().len());
    for name in spec.required_environment() {
        writer.put_string(name.as_str());
    }
    writer.put_count(spec.secret_environment().len());
    for name in spec.secret_environment() {
        writer.put_string(name.as_str());
    }
    writer.put_count(spec.sealed_environment().len());
    for (name, value) in spec.sealed_environment() {
        writer.put_string(name.as_str());
        writer.put_string(value);
    }
    writer.put_count(spec.destinations().len());
    for destination in spec.destinations() {
        writer.put_string(destination.host());
        writer.put_u16(destination.port());
    }
    match spec.health_probe() {
        None => writer.put_u8(0),
        Some(HealthProbe::Command {
            program,
            args,
            timeout_seconds,
        }) => {
            writer.put_u8(1);
            writer.put_string(program);
            writer.put_strings(args);
            writer.put_u32(*timeout_seconds);
        }
        Some(HealthProbe::Tcp { port }) => {
            writer.put_u8(2);
            writer.put_u16(*port);
        }
    }
    writer.put_count(spec.platforms().len());
    for platform in spec.platforms() {
        writer.put_string(platform.operating_system());
        writer.put_string(platform.architecture());
        writer.put_optional_string(platform.variant());
    }
    writer.put_presence(spec.default_command().is_some());
    if let Some(command) = spec.default_command() {
        put_command(&mut writer, command);
    }
    Sha256::digest(writer.finish()).into()
}

pub(crate) fn put_command(writer: &mut Writer, command: &Command) {
    writer.put_string(command.program());
    writer.put_strings(command.args());
    writer.put_optional_string(command.working_directory());
    writer.put_optional_string(command.user());
}
