use wasm::{components, web};

fn main() {
    console_error_panic_hook::set_once();
    if let Err(e) = web::theme::apply_theme(&web::cookies::get_cookie_theme()) {
        web_sys::console::error_1(&format!("Could not apply theme before startup: {e:?}").into());
    }
    leptos::mount::mount_to_body(components::app::App);
}
