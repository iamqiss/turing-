use dioxus::prelude::*;
use crate::logo::TtMark;
use crate::Route;

#[component]
pub fn Sidebar() -> Element {
    rsx! {
        aside {
            style: "width:var(--tt-sidebar-w);min-height:100vh;background:var(--tt-black);color:#fafaf7;padding:16px;",
            div {
                style: "display:flex;align-items:center;gap:10px;margin-bottom:20px;",
                TtMark { size: 34 }
                span { style: "font-weight:700;", "Turing Tarpit" }
            }
            Link { to: Route::Projects {}, style: "display:block;color:#ff3c00;padding:8px 0;", "Projects" }
            Link { to: Route::Inbox {}, style: "display:block;color:#d4d4d8;padding:8px 0;", "Inbox" }
            Link { to: Route::Agents {}, style: "display:block;color:#d4d4d8;padding:8px 0;", "Agents" }
        }
    }
}
