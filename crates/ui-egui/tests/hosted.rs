//! Hosted mode (the web build in another app's frame): the user can't swap the host's workbook,
//! the host's own open can, and a password-protected or broken file is reported, not shown.

use gridcraft_engine::{Session, core::CellRef, core::Value};
use gridcraft_ui_egui::{Services, SheetApp, hosted_blocks};
use serde_json::json;

fn app(hosted: bool) -> SheetApp {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Services::default());
    app.hosted = hosted;
    app
}

/// A workbook with `text` in A1, saved as `.xlsx`.
fn xlsx_with(text: &str) -> Vec<u8> {
    let mut s = Session::new();
    s.new_workbook();
    s.run("cell.set", json!({"cell": "A1", "input": text})).unwrap();
    gridcraft_engine::io::save_bytes(&s.active().unwrap().wb, "book.xlsx").unwrap()
}

fn a1(app: &SheetApp) -> Value {
    app.session.active().unwrap().wb.active().unwrap().value(CellRef::parse("A1").unwrap())
}

#[test]
fn hosted_the_user_cannot_swap_the_workbook() {
    let mut a = app(true);
    for id in ["file.new", "file.open", "file.close"] {
        assert!(hosted_blocks(id), "{id}");
        let err = a.run(id, json!({})).unwrap_err();
        assert!(err.contains("is not available right now"), "{id}: {err}");
    }
    // Opening with a file named is refused too, and nothing changed.
    assert!(a.run("file.open", json!({"name": "x.xlsx", "base64": gridcraft_engine::io::base64_encode(&xlsx_with("1"))})).is_err());
    assert_eq!(a.session.documents().len(), 1);
    // The rest, saving among it, still runs.
    assert!(a.run("file.save", json!({})).is_ok());
    assert!(!hosted_blocks("file.saveAs") && !hosted_blocks("file.save") && !hosted_blocks("edit.undo"));
}

#[test]
fn outside_a_host_nothing_is_refused() {
    let mut a = app(false);
    assert!(a.run("file.new", json!({})).is_ok());
    assert_eq!(a.session.documents().len(), 2);
    assert!(a.run("file.close", json!({"force": true})).is_ok());
}

#[test]
fn the_host_can_still_open_and_run_what_the_user_cannot() {
    let mut a = app(true);
    a.open_bytes("Plan.xlsx", &xlsx_with("42")).unwrap();
    assert_eq!(a.session.active().unwrap().title, "Plan.xlsx");
    assert_eq!(a1(&a), Value::Number(42.0));
    assert!(a.execute("file.new", json!({})).is_ok(), "execute is never limited");
}

#[test]
fn an_open_asks_nothing_about_unsaved_changes() {
    let mut a = app(true);
    a.run("cell.set", json!({"cell": "A1", "input": "unsaved"})).unwrap();
    assert!(a.session.active().unwrap().is_dirty());
    a.open_bytes("Next.xlsx", &xlsx_with("7")).unwrap();
    assert!(a.dialog.is_none(), "no Save changes box");
    assert!(a.message.is_none());
    assert_eq!(a1(&a), Value::Number(7.0));
}

#[test]
fn a_file_that_is_not_a_workbook_fails_and_leaves_the_open_one() {
    let mut a = app(true);
    a.open_bytes("Plan.xlsx", &xlsx_with("42")).unwrap();
    // A zip header with nothing behind it. (Plain text named .xlsx opens as text, as upstream decides.)
    assert!(a.open_bytes("Broken.xlsx", b"PK not a package").is_err());
    assert!(a.open_bytes("Legacy.xls", &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0]).is_err());
    assert!(a.message.is_none(), "the failure is returned, not shown");
    assert_eq!(a.session.documents().len(), 1);
    assert_eq!(a1(&a), Value::Number(42.0));
}

#[test]
fn a_password_protected_workbook_is_reported_as_such() {
    let mut a = app(true);
    let encrypted = gridcraft_xlsx::testing::encrypt_workbook(&xlsx_with("42"), "synthetic-pw-8842");
    let err = a.open_bytes("Secret.xlsx", &encrypted).unwrap_err();
    assert!(err.contains(gridcraft_engine::io::PASSWORD_REQUIRED), "{err}");
    assert!(a.dialog.is_none(), "open_bytes asks for no password itself");
}

#[test]
fn saving_hands_the_workbook_over_and_counts_as_saved() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let got: Rc<RefCell<Vec<(String, usize)>>> = Rc::default();
    let sink = got.clone();
    let mut a = app(true);
    a.services.download = Some(Box::new(move |name, bytes| sink.borrow_mut().push((name.to_string(), bytes.len()))));
    a.open_bytes("Plan.xlsx", &xlsx_with("1")).unwrap();
    a.run("cell.set", json!({"cell": "B2", "input": "9"})).unwrap();
    a.run("file.save", json!({})).unwrap();
    let got = got.borrow();
    assert_eq!(got.len(), 1, "one file handed over");
    assert_eq!(got[0].0, "Plan.xlsx");
    assert!(got[0].1 > 100);
}
