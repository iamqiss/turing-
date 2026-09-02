use dioxus::prelude::*;
use crate::components::topbar::TopBar;

#[component]
pub fn InboxPage() -> Element {
    rsx! {
        div {
            style: "padding:28px;",
            TopBarTopBar { title: "Inbox".into(), subtitle: None }
        }
    }
}
