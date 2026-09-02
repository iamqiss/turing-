use dioxus::prelude::*;
use crate::components::topbar::TopBar;

#[component]
pub fn AgentsPage() -> Element {
    rsx! {
        div {
            style: "padding:28px;",
            TopBar { title: "Agents".into(), subtitle: Some("Pearl".into()) }
        }
    }
}
