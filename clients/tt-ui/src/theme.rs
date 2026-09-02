pub const ORANGE: &str = "#ff3c00";
pub const ORANGE_HOVER: &str = "#e63500";
pub const ORANGE_SOFT: &str = "rgba(255, 60, 0, 0.12)";
pub const BLACK: &str = "#0a0a0a";
pub const BLACK_ELEVATED: &str = "#141414";
pub const GREY: &str = "#6b7280";
pub const GREY_BORDER: &str = "#e5e7eb";
pub const IVORY: &str = "#fafaf7";
pub const MINT: &str = "#c4ebcc";
pub const SURFACE: &str = "#ffffff";
pub const SURFACE_MUTED: &str = "#f4f4f5";
pub const TEXT: &str = "#18181b";
pub const TEXT_MUTED: &str = "#71717a";
pub const SIDEBAR_W: &str = "260px";

pub fn global_css() -> String {
    format!(
        r#"
        :root {{
            --tt-orange: {ORANGE};
            --tt-orange-hover: {ORANGE_HOVER};
            --tt-orange-soft: {ORANGE_SOFT};
            --tt-black: {BLACK};
            --tt-black-elevated: {BLACK_ELEVATED};
            --tt-grey: {GREY};
            --tt-border: {GREY_BORDER};
            --tt-ivory: {IVORY};
            --tt-mint: {MINT};
            --tt-surface: {SURFACE};
            --tt-surface-muted: {SURFACE_MUTED};
            --tt-text: {TEXT};
            --tt-text-muted: {TEXT_MUTED};
            --tt-sidebar-w: {SIDEBAR_W};
            --tt-radius: 12px;
            --tt-radius-sm: 8px;
            --tt-font: "Inter", system-ui, -apple-system, sans-serif;
        }}
        * {{ box-sizing: border-box; }}
        html, body {{
            margin: 0; padding: 0; height: 100%;
            font-family: var(--tt-font);
            color: var(--tt-text);
            background: var(--tt-surface-muted);
        }}
        button {{ font-family: inherit; cursor: pointer; }}
        a {{ color: inherit; text-decoration: none; }}
        "#
    )
}
