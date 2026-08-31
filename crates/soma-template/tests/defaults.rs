//! Every table and field that may be omitted, and what omitting it means.
//!
//! The point of these tests is that the documented default is the only default. A silent
//! disagreement between the schema and the rest of the product would be worse than the
//! verbosity the defaults remove, so the Machine shape assertions read the `MachineShape`
//! constants the command line defaults to rather than restating the numbers.

mod support;

use soma::MachineShape;
use soma_template::{
    DEFAULT_IDLE_TIMEOUT_SECONDS, DEFAULT_MAXIMUM_LIFETIME_SECONDS, DEFAULT_ON_IDLE, IdleAction,
    Lifecycle, Resources,
};
use support::{lock, parse};

/// The five values that have no default: schema, name, image, platform, and program.
const MINIMUM: &str = r#"schema = "soma.template/v1alpha1"
name = "minimum"

[workload]
image = "python:3.12-slim"
platform = "linux/amd64"

[command]
program = "/bin/sh"
"#;

#[test]
fn the_minimum_document_is_five_values_and_it_resolves() {
    let template = parse(MINIMUM);
    assert_eq!(template.name(), "minimum");
    assert!(template.command().expect("command").args().is_empty());
    lock(MINIMUM);
}

#[test]
fn an_omitted_resources_table_is_the_command_line_default_shape() {
    let resources = *parse(MINIMUM).resources();
    assert_eq!(resources, Resources::default());
    assert_eq!(resources.vcpus, u32::from(MachineShape::DEFAULT_VCPU_COUNT));
    assert_eq!(resources.memory_mib, MachineShape::DEFAULT_MEMORY_MIB);
    assert_eq!(
        resources.writable_storage_mib,
        MachineShape::DEFAULT_STORAGE_MIB
    );
}

#[test]
fn an_omitted_lifecycle_table_is_five_idle_minutes_and_one_hour_of_life() {
    let lifecycle = *parse(MINIMUM).lifecycle();
    assert_eq!(lifecycle, Lifecycle::default());
    assert_eq!(lifecycle.idle_timeout_seconds, DEFAULT_IDLE_TIMEOUT_SECONDS);
    assert_eq!(
        lifecycle.maximum_lifetime_seconds,
        DEFAULT_MAXIMUM_LIFETIME_SECONDS
    );
    assert_eq!(lifecycle.on_idle, DEFAULT_ON_IDLE);
    assert_eq!(DEFAULT_IDLE_TIMEOUT_SECONDS, 300);
    assert_eq!(DEFAULT_MAXIMUM_LIFETIME_SECONDS, 3_600);
    assert_eq!(DEFAULT_ON_IDLE, IdleAction::Destroy);
}

#[test]
fn a_stated_value_wins_over_every_default() {
    let text = format!(
        "{MINIMUM}\n[resources]\nvcpus = 4\nmemory_mib = 2048\nwritable_storage_mib = 4096\n\
         [lifecycle]\nidle_timeout_seconds = 60\nmaximum_lifetime_seconds = 120\non_idle = \"stop\"\n"
    );
    let template = parse(&text);
    assert_eq!(
        *template.resources(),
        Resources {
            vcpus: 4,
            memory_mib: 2048,
            writable_storage_mib: 4096,
        }
    );
    assert_eq!(
        *template.lifecycle(),
        Lifecycle {
            idle_timeout_seconds: 60,
            maximum_lifetime_seconds: 120,
            on_idle: IdleAction::Stop,
        }
    );
}

#[test]
fn a_partial_table_defaults_only_the_fields_it_leaves_out() {
    let text =
        format!("{MINIMUM}\n[resources]\nmemory_mib = 8192\n[lifecycle]\non_idle = \"stop\"\n");
    let template = parse(&text);
    let resources = *template.resources();
    assert_eq!(resources.memory_mib, 8192);
    assert_eq!(resources.vcpus, Resources::default().vcpus);
    assert_eq!(
        resources.writable_storage_mib,
        Resources::default().writable_storage_mib
    );
    let lifecycle = *template.lifecycle();
    assert_eq!(lifecycle.on_idle, IdleAction::Stop);
    assert_eq!(lifecycle.idle_timeout_seconds, DEFAULT_IDLE_TIMEOUT_SECONDS);
    assert_eq!(
        lifecycle.maximum_lifetime_seconds,
        DEFAULT_MAXIMUM_LIFETIME_SECONDS
    );
}

#[test]
fn an_empty_table_means_exactly_the_same_as_an_omitted_one() {
    let text = format!("{MINIMUM}\n[resources]\n[lifecycle]\n[network]\n");
    let template = parse(&text);
    let omitted = parse(MINIMUM);
    assert_eq!(*template.resources(), *omitted.resources());
    assert_eq!(*template.lifecycle(), *omitted.lifecycle());
    assert_eq!(*template.network(), *omitted.network());
}

#[test]
fn an_omitted_args_list_is_no_arguments_and_matches_an_empty_one() {
    let stated = format!("{MINIMUM}args = []\n");
    assert_eq!(
        parse(&stated).command().expect("command").args(),
        parse(MINIMUM).command().expect("command").args()
    );
    assert_eq!(lock(&stated).id(), lock(MINIMUM).id());
}
