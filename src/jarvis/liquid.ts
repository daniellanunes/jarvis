// "Vidro Líquido" (liquid.css): the SVG filters that bend what lies behind
// the glass, and the mouse position for the specular light on the rims.

let installed = false;
let raf = 0;
let pending: { x: number; y: number } | null = null;

/** The refraction filters, once per page (referenced as url(#jv-liquid…)). */
function installFilters() {
  if (document.getElementById("jv-liquid-defs")) return;
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.id = "jv-liquid-defs";
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("width", "0");
  svg.setAttribute("height", "0");
  svg.style.position = "absolute";
  // A soft, slow noise moves the pixels behind the glass a few px: a lens.
  svg.innerHTML = `
    <filter id="jv-liquid" x="-10%" y="-10%" width="120%" height="120%" color-interpolation-filters="sRGB">
      <feTurbulence type="fractalNoise" baseFrequency="0.006 0.01" numOctaves="2" seed="7" result="noise"/>
      <feGaussianBlur in="noise" stdDeviation="2.5" result="soft"/>
      <feDisplacementMap in="SourceGraphic" in2="soft" scale="22" xChannelSelector="R" yChannelSelector="G"/>
    </filter>
    <filter id="jv-liquid-strong" x="-10%" y="-10%" width="120%" height="120%" color-interpolation-filters="sRGB">
      <feTurbulence type="fractalNoise" baseFrequency="0.012 0.018" numOctaves="1" seed="3" result="noise"/>
      <feGaussianBlur in="noise" stdDeviation="1.5" result="soft"/>
      <feDisplacementMap in="SourceGraphic" in2="soft" scale="30" xChannelSelector="R" yChannelSelector="G"/>
    </filter>`;
  document.body.append(svg);
}

/** Follows the mouse (one style write per frame at most). */
function onMove(e: PointerEvent) {
  if (!document.documentElement.classList.contains("liquid")) return;
  pending = { x: e.clientX, y: e.clientY };
  if (raf) return;
  raf = requestAnimationFrame(() => {
    raf = 0;
    if (!pending) return;
    const s = document.documentElement.style;
    s.setProperty("--mx", `${pending.x}px`);
    s.setProperty("--my", `${pending.y}px`);
  });
}

/** Turns the liquid glass on or off (the classes are set by applyTheme). */
export function setLiquidGlass(on: boolean) {
  if (on && !installed) {
    installed = true;
    installFilters();
    window.addEventListener("pointermove", onMove, { passive: true });
    // The mouse left the window: the light rests at the top centre.
    document.addEventListener("pointerleave", () => {
      const s = document.documentElement.style;
      s.setProperty("--mx", "50vw");
      s.setProperty("--my", "-120px");
    });
  }
}
