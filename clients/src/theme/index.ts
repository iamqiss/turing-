export const theme = {
colors: {
orange: '#ff3c00',
/** True AMOLED black — pure #000 for OLED power + contrast */
pitchBlack: '#000000',
background: '#000000',
surface: '#0a0a0a',
surfaceElevated: '#121212',
border: '#1f1f1f',
text: '#ffffff',
textMuted: '#8a8a8a',
textDim: '#5c5c5c',
success: '#22c55e',
danger: '#ef4444',
warning: '#f59e0b',
inputBg: '#0a0a0a',
inputBorder: '#222222',
inputBorderFocus: '#ff3c00',
},
spacing: { xs: 4, sm: 8, md: 16, lg: 24, xl: 32, xxl: 48 },
radius: { sm: 8, md: 12, lg: 16, full: 9999 },
typography: {
hero: { fontSize: 32, fontWeight: '700' as const, letterSpacing: -0.5 },
title: { fontSize: 22, fontWeight: '600' as const },
subtitle: { fontSize: 16, fontWeight: '500' as const },
body: { fontSize: 16, fontWeight: '400' as const },
caption: { fontSize: 13, fontWeight: '400' as const },
label: { fontSize: 12, fontWeight: '600' as const, letterSpacing: 0.4 },
},
} as const;

export type Theme = typeof theme;
