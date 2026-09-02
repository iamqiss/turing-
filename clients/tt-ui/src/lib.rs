//! Shared Turing Tarpit Dioxus UI.

pub mod api;
pub mod components;
pub mod logo;
pub mod pages;
pub mod theme;

use dioxus::prelude::*;

use components::sidebar::Sidebar;
use pages::agents::AgentsPage;
use pages::inbox::InboxPage;
use pages::projects::ProjectsPage;

#[derive(Clone, Routable, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(AppShell)]
        #[route("/")]
        Projects {},
        #[route("/inbox")]
        Inbox {},
        #[route("/agents")]
        Agents {},
}

#[component]
fn AppShell() -> Element {
    rsx! {
        style { { theme::global_css() } }
        div {
            style: "display:flex;min-height:100vh;background:var(--tt-surface-muted);",
            Sidebar {}
            main {
                style: "flex:1;min-width:0;background:var(--tt-surface-muted);",
                Outlet::<Route> {}
            }
        }
    }
}

#[component]
pub fn Projects() -> Element {
    rsx! { ProjectsPage {} }
}

#[component]
pub fn Inbox() -> Element {
    rsx! { InboxPage {} }
}

#[component]
pub fn Agents() -> Element {
    rsx! { AgentsPage {} }
}

#[component]
pub fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}
