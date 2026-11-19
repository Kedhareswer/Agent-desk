// The glow window only listens for "glow" on/off; it has no app commands.
const glow = document.getElementById("glow");
// Payload: "listening", "acting" (Agent Desk is working on the computer), or
// false. Older callers sent true for listening.
window.__TAURI__.event.listen("glow", ({ payload }) => {
  glow.classList.toggle("on", !!payload);
  glow.classList.toggle("acting", payload === "acting");
});
