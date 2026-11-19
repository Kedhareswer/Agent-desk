// The glow window only listens for "glow" on/off; it has no app commands.
const glow = document.getElementById("glow");
window.__TAURI__.event.listen("glow", ({ payload }) => glow.classList.toggle("on", !!payload));
