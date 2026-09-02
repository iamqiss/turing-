use dioxus::prelude::*;
use crate::components::topbar::TopBar;

#[component]
pub fn ProjectsPage() -> Element {
    rsx! {
        div {
            style: "padding:28px;",
            TopBar { title: "Projects".into(), subtitle: Some("Set up and manage your projects.".into()) }
            p { style: "color:var(--tt-text-muted);", "Activity and repos wire to the API next." }
        }
    }
}
