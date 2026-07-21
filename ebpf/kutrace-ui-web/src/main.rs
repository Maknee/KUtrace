mod api;
mod app;
mod model;
mod timeline;

fn main() {
    yew::Renderer::<app::App>::new().render();
}
