// A small, safe Markdown renderer: everything is HTML-escaped first, and
// only a fixed set of tags is ever produced.
(function () {
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

  function inline(text) {
    const codes = [];
    let out = esc(text).replace(/`([^`\n]+)`/g, (_, c) => {
      codes.push(c);
      return `\u0000${codes.length - 1}\u0000`;
    });
    out = out
      .replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
      .replace(/(^|[\s(])\*([^*\s][^*\n]*)\*(?=[\s).,!?:;]|$)/g, "$1<em>$2</em>")
      .replace(/\[([^\]\n]+)\]\((https?:[^)\s]+)\)/g, '$1 (<span class="link">$2</span>)');
    return out.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${codes[i]}</code>`);
  }

  function blocks(text) {
    const html = [];
    let para = [];
    let list = null;
    const flushPara = () => {
      if (para.length) html.push(`<p>${para.map(inline).join("<br>")}</p>`);
      para = [];
    };
    const flushList = () => {
      if (list) html.push(`<${list.tag}>${list.items.map((i) => `<li>${inline(i)}</li>`).join("")}</${list.tag}>`);
      list = null;
    };
    for (const line of text.split("\n")) {
      const heading = /^(#{1,4})\s+(.*)$/.exec(line);
      const bullet = /^\s*[-*•]\s+(.*)$/.exec(line);
      const numbered = /^\s*\d+[.)]\s+(.*)$/.exec(line);
      if (heading) {
        flushPara(); flushList();
        html.push(`<h3>${inline(heading[2])}</h3>`);
      } else if (bullet || numbered) {
        flushPara();
        const tag = bullet ? "ul" : "ol";
        if (!list || list.tag !== tag) { flushList(); list = { tag, items: [] }; }
        list.items.push((bullet || numbered)[1]);
      } else if (!line.trim()) {
        flushPara(); flushList();
      } else {
        flushList();
        para.push(line);
      }
    }
    flushPara(); flushList();
    return html.join("");
  }

  window.renderMarkdown = function (text) {
    // Split on fenced code blocks; an unclosed fence (still streaming) is
    // treated as code to the end.
    const parts = text.split(/```/);
    return parts
      .map((part, i) => {
        if (i % 2 === 0) return blocks(part);
        const nl = part.indexOf("\n");
        const code = nl === -1 ? part : part.slice(nl + 1);
        return `<pre><code>${esc(code.replace(/\n$/, ""))}</code></pre>`;
      })
      .join("");
  };
})();
