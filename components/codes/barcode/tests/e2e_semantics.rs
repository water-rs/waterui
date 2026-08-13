//! End-to-end accessibility-semantics tests for the `barcode` component.

use waterui::ViewExt as _;
use waterui_barcode::Barcode;
use waterui_testing::{Role, SemanticApp};

fn qr_barcode_view() -> impl waterui::View {
    Barcode::qr("https://waterui.dev/testing").size(180.0, 180.0)
}

#[waterui::test(qr_barcode_view)]
fn qr_barcode_exposes_accessible_image_node(app: &mut SemanticApp) {
    let node = app
        .query()
        .role(Role::IMAGE)
        .label("QR code: https://waterui.dev/testing")
        .single();
    let bounds = node.bounds();
    assert!(bounds.width() > 0.0, "barcode-qr: width must be positive");
    assert!(bounds.height() > 0.0, "barcode-qr: height must be positive");
}

fn code128_barcode_view() -> impl waterui::View {
    Barcode::code128("HELLO-WATERUI-128").size(180.0, 80.0)
}

#[waterui::test(code128_barcode_view)]
fn code128_barcode_exposes_accessible_image_node(app: &mut SemanticApp) {
    let node = app
        .query()
        .role(Role::IMAGE)
        .label("Code 128 barcode: HELLO-WATERUI-128")
        .single();
    let bounds = node.bounds();
    assert!(
        bounds.width() > 0.0,
        "barcode-code128: width must be positive"
    );
    assert!(
        bounds.height() > 0.0,
        "barcode-code128: height must be positive"
    );
}
