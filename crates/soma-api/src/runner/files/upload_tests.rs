//! The upload form's destination rules, event by event.

use super::{Destination, MAX_UPLOAD_FILE_BYTES, Step, UploadForm};
use crate::runner::files::{multipart::Event, params::Params};

fn form_with(query: Option<&str>) -> UploadForm {
    UploadForm::new(&Params::read(query, b"").expect("params"))
}

fn field(form: &mut UploadForm, name: &str, value: &str) {
    form.on(Event::Begin {
        name: name.to_owned(),
        filename: None,
    })
    .expect("begins");
    form.on(Event::Data(value.as_bytes().to_vec()))
        .expect("data");
    form.on(Event::End).expect("ends");
}

fn file(form: &mut UploadForm, filename: Option<&str>) -> Destination {
    match form.on(Event::Begin {
        name: "file".to_owned(),
        filename: filename.map(str::to_owned),
    }) {
        Ok(Step::Begin(destination)) => destination,
        other => panic!("the file part names its destination: {other:?}"),
    }
}

#[test]
fn a_directory_path_receives_the_file_under_its_own_name() {
    let mut form = form_with(None);
    field(&mut form, "path", "/workspace/in/");
    let destination = file(&mut form, Some("C:/fakepath/report.csv"));
    assert_eq!(destination.target.as_str(), "/workspace/in/report.csv");
    assert_eq!(destination.echo, "/workspace/in/report.csv");
    assert_eq!(destination.filename, "report.csv");
}

#[test]
fn a_file_path_is_the_destination_itself_and_the_default_is_the_guest_home() {
    let mut form = form_with(Some("path=/tmp/exact.bin"));
    assert_eq!(file(&mut form, Some("x")).target.as_str(), "/tmp/exact.bin");

    let mut form = form_with(None);
    let destination = file(&mut form, None);
    assert_eq!(destination.target.as_str(), "/home/user/upload");
}

#[test]
fn a_form_field_wins_over_the_query_as_phoenix_merges_them() {
    let mut form = form_with(Some("path=/tmp/query/"));
    field(&mut form, "path", "/workspace/field/");
    assert_eq!(
        file(&mut form, Some("a")).target.as_str(),
        "/workspace/field/a"
    );
}

#[test]
fn a_destination_the_contract_does_not_expose_is_refused() {
    for (path, filename) in [("/etc/", "passwd"), ("/tmp/", ".."), ("/root/x", "y")] {
        let mut form = form_with(None);
        field(&mut form, "path", path);
        let refused = form
            .on(Event::Begin {
                name: "file".to_owned(),
                filename: Some(filename.to_owned()),
            })
            .expect_err("refused");
        assert_eq!(refused.code, "INVALID_PATH", "{path}{filename}");
    }
}

#[test]
fn a_path_named_after_the_file_part_is_refused_unless_it_agrees() {
    let mut form = form_with(None);
    field(&mut form, "path", "/tmp/");
    file(&mut form, Some("a"));
    form.on(Event::End).expect("the file part ends");
    field(&mut form, "path", "/tmp/");

    let mut late = super::UploadForm::new(&Params::default());
    file(&mut late, Some("a"));
    late.on(Event::End).expect("the file part ends");
    late.on(Event::Begin {
        name: "path".to_owned(),
        filename: None,
    })
    .expect("begins");
    late.on(Event::Data(b"/workspace/".to_vec())).expect("data");
    let refused = late.on(Event::End).expect_err("too late");
    assert_eq!(
        refused.message,
        "the path field must come before the file part"
    );
}

#[test]
fn a_file_larger_than_an_upload_moves_is_refused_as_it_arrives() {
    let mut form = form_with(None);
    file(&mut form, Some("big"));
    let chunk = vec![0_u8; 64 * 1024 * 1024];
    let mut sent = 0;
    let refused = loop {
        match form.on(Event::Data(chunk.clone())) {
            Ok(Step::Bytes(_)) => sent += chunk.len() as u64,
            Ok(other) => panic!("{other:?}"),
            Err(refused) => break refused,
        }
    };
    assert!(sent <= MAX_UPLOAD_FILE_BYTES);
    assert_eq!(
        (refused.status, refused.message.as_str()),
        (413, "file exceeds 500 MiB limit")
    );
}

#[test]
fn other_fields_are_ignored_and_a_long_path_field_is_refused() {
    let mut form = form_with(None);
    field(&mut form, "mode", "0755");
    form.on(Event::Begin {
        name: "path".to_owned(),
        filename: None,
    })
    .expect("begins");
    let refused = form
        .on(Event::Data(vec![b'a'; 9 * 1024]))
        .expect_err("too long");
    assert_eq!(refused.code, "INVALID_BODY");
}
