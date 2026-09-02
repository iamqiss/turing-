use dioxus::prelude::*;

#[component]
pub fn StatCard(label: String, value: String, icon: String) -> Element {
    let _ = icon;
    rsx! {
        div {
            style: "background:var(--tt-surface);border:1px solid var(--tt-border);border-radius:12px;padding:14px;",
            div { style: "color:var(--tt-text-muted);font-size:0.8rem;", "{label}" }
            div { style: "font-size:1.4rem;font-weight:700;", "{value}" }
        }
    }
}
