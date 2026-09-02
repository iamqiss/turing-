use dioxus::prelude::*;

#[component]
pub fn TopBar(title: String, subtitle: Option<String>) -> Element {
    rsx! {
        div {
            h1 { style: "margin:0;font-size:1.6rem;font-weight:700;", "{title}" }
            if let Some(sub) = subtitle {
                p { style: "margin:6px 0 0;color:var(--tt-text-muted);", "{sub}" }
            }
        }
    }
}
