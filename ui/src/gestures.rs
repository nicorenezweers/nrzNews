//! Touch-Gesten fürs Handy, in JavaScript (flüssig, ohne Umweg über WebAssembly bei jeder
//! Fingerbewegung). Die Listener hängen einmal am Dokument; was dabei herauskommt, geht als
//! Nachricht an die Seite, die gerade zuhört (`sink`):
//!
//! * Artikelzeile (`[data-swipe-id]`) nach rechts wischen → `{t:"swipe", id, action:"read"}`,
//!   nach links → `{…, action:"star"}`
//! * Liste (`#article-list`) oben herunterziehen → `{t:"pull"}`
//! * Lesemodus (`#reader-swipe`, nur schmale Bildschirme) nach links → `{t:"next"}`,
//!   nach rechts → `{t:"prev"}`; die Ränder bleiben frei für die Zurück-Geste des Browsers
//! * Seitenleiste (`#mobile-sheet`) nach links wischen → `{t:"close-sheet"}`
//!
//! Dazu der Lesefortschritt (`#reader-progress`) beim Scrollen im Lesemodus.

use dioxus::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Gesture {
    pub t: String,
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub action: Option<String>,
}

const SCRIPT: &str = r#"
const sinks = (window.__nrznewsSinks ||= {});
sinks[__SINK__] = (m) => { try { dioxus.send(m); } catch (_) {} };
if (!window.__nrznewsGestures) {
  window.__nrznewsGestures = true;
  const send = (name, m) => { const s = sinks[name]; if (s) s(m); };
  const buzz = () => { try { navigator.vibrate && navigator.vibrate(8); } catch (_) {} };
  const EDGE = 24;
  let g = null;
  let noClickUntil = 0;

  const scrollsX = (el, stop) => {
    for (; el && el !== stop; el = el.parentElement) {
      if (el.scrollWidth > el.clientWidth + 2) {
        const o = getComputedStyle(el).overflowX;
        if (o === 'auto' || o === 'scroll') return true;
      }
    }
    return false;
  };
  const arm = (el, on) => {
    if (!el) return false;
    const was = el.hasAttribute('data-armed');
    if (on && !was) { el.setAttribute('data-armed', ''); buzz(); }
    if (!on && was) el.removeAttribute('data-armed');
    return on;
  };
  const settle = (el, done) => {
    if (!el) return;
    el.classList.add('is-settling');
    el.style.transform = '';
    el.style.opacity = '';
    setTimeout(() => { el.classList.remove('is-settling'); done && done(); }, 230);
  };
  const hint = (name) => document.getElementById('swipe-hint-' + name);
  const resetHints = () => ['prev', 'next'].forEach((n) => {
    const h = hint(n);
    if (h) { h.style.opacity = ''; h.style.transform = ''; h.removeAttribute('data-armed'); }
  });

  document.addEventListener('touchstart', (e) => {
    if (e.touches.length !== 1) { g = null; return; }
    const t = e.touches[0];
    const el = e.target instanceof Element ? e.target : null;
    if (!el) { g = null; return; }
    const reader = el.closest('#reader-swipe');
    g = {
      x0: t.clientX, y0: t.clientY, dx: 0, dy: 0, mode: null, armed: false,
      row: el.closest('[data-swipe-id]'),
      list: el.closest('#article-list'),
      sheet: el.closest('#mobile-sheet'),
      reader: reader && innerWidth < 768 && t.clientX > EDGE && t.clientX < innerWidth - EDGE
        && !el.closest('pre, table, video, iframe, audio, input, textarea, select') && !scrollsX(el, reader)
        ? reader : null,
    };
  }, { passive: true });

  document.addEventListener('touchmove', (e) => {
    if (!g || e.touches.length !== 1) return;
    const t = e.touches[0];
    g.dx = t.clientX - g.x0;
    g.dy = t.clientY - g.y0;
    if (!g.mode) {
      const ax = Math.abs(g.dx), ay = Math.abs(g.dy);
      if (ax < 8 && ay < 8) return;
      if (ax > ay * 1.2) g.mode = g.row ? 'row' : g.reader ? 'reader' : g.sheet ? 'sheet' : 'none';
      else g.mode = g.list && g.list.scrollTop <= 0 && g.dy > 0 && document.getElementById('ptr') ? 'pull' : 'none';
      if (g.mode === 'none') { g = null; return; }
      // ab hier zählt die Bewegung (kein Sprung um die Schwelle)
      g.x0 += Math.sign(g.dx) * Math.min(ax, 8);
      g.y0 += Math.sign(g.dy) * Math.min(ay, 8);
      g.dx = t.clientX - g.x0;
      g.dy = t.clientY - g.y0;
      if (g.mode === 'row') g.fg = g.row.querySelector('.swipe-fg');
      if (g.mode === 'pull') g.ptr = document.getElementById('ptr');
    }
    if (e.cancelable) e.preventDefault();
    if (g.mode === 'row') {
      if (!g.fg) return;
      const w = g.row.offsetWidth, lim = Math.min(w * 0.4, 160), a = Math.abs(g.dx);
      const x = a <= lim ? g.dx : Math.sign(g.dx) * (lim + (a - lim) * 0.25);
      g.fg.style.transform = 'translateX(' + x + 'px)';
      if (g.dx !== 0) g.row.dataset.dir = g.dx > 0 ? 'right' : 'left';
      g.armed = arm(g.row, a > Math.min(w * 0.25, 96));
    } else if (g.mode === 'reader') {
      const name = g.dx < 0 ? 'next' : 'prev', other = g.dx < 0 ? 'prev' : 'next';
      const h = hint(name), o = hint(other);
      if (o) { o.style.opacity = ''; o.removeAttribute('data-armed'); }
      const a = Math.abs(g.dx), p = Math.min(a / 90, 1);
      g.reader.style.transform = 'translateX(' + (h ? g.dx * 0.35 : g.dx * 0.12) + 'px)';
      g.reader.style.opacity = String(1 - Math.min(a / 700, 0.25));
      if (h) {
        h.style.opacity = String(p);
        h.style.transform = 'translateY(-50%) scale(' + (0.8 + p * 0.2) + ')';
      }
      g.armed = arm(h, !!h && a > 90);
    } else if (g.mode === 'sheet') {
      const x = Math.min(0, g.dx);
      g.sheet.style.transform = 'translateX(' + x + 'px)';
      g.armed = x < -70;
    } else if (g.mode === 'pull') {
      const h = Math.max(0, Math.min(g.dy * 0.5, 96));
      g.ptr.classList.add('is-pulling');
      g.ptr.style.height = h + 'px';
      g.armed = arm(g.ptr, h >= 56);
    }
  }, { passive: false });

  const end = (cancelled) => {
    const s = g;
    g = null;
    if (!s || !s.mode) return;
    noClickUntil = Date.now() + 350;
    const ok = s.armed && !cancelled;
    if (s.mode === 'row') {
      const dir = s.row.dataset.dir;
      settle(s.fg, () => { delete s.row.dataset.dir; s.row.removeAttribute('data-armed'); });
      if (ok) send('list', { t: 'swipe', id: Number(s.row.dataset.swipeId), action: dir === 'right' ? 'read' : 'star' });
    } else if (s.mode === 'reader') {
      settle(s.reader);
      resetHints();
      if (ok) send('list', { t: s.dx < 0 ? 'next' : 'prev' });
    } else if (s.mode === 'sheet') {
      if (ok) {
        s.sheet.style.transform = '';
        send('shell', { t: 'close-sheet' });
      } else settle(s.sheet);
    } else if (s.mode === 'pull') {
      s.ptr.classList.remove('is-pulling');
      s.ptr.removeAttribute('data-armed');
      if (ok) {
        send('list', { t: 'pull' });
        setTimeout(() => { s.ptr.style.height = ''; }, 400);
      } else s.ptr.style.height = '';
    }
  };
  document.addEventListener('touchend', () => end(false), { passive: true });
  document.addEventListener('touchcancel', () => end(true), { passive: true });
  // Nach einer Geste keinen Klick auslösen (sonst öffnet die Zeile den Artikel)
  document.addEventListener('click', (e) => {
    if (Date.now() < noClickUntil) { e.preventDefault(); e.stopPropagation(); }
  }, true);

  // Lesefortschritt
  document.addEventListener('scroll', (e) => {
    const el = e.target;
    if (!(el instanceof Element) || el.id !== 'reader-scroll') return;
    const bar = document.getElementById('reader-progress');
    if (!bar) return;
    const max = el.scrollHeight - el.clientHeight;
    bar.style.width = (max > 0 ? Math.min(100, (el.scrollTop / max) * 100) : 0) + '%';
  }, { capture: true, passive: true });
}
await new Promise(() => {});
"#;

/// Gesten anmelden; die Nachrichten für `sink` ("list" oder "shell") kommen über `on`.
pub fn listen(sink: &'static str, mut on: impl FnMut(Gesture) + 'static) -> impl std::future::Future<Output = ()> {
    async move {
        let mut eval = document::eval(&SCRIPT.replace("__SINK__", &format!("{sink:?}")));
        while let Ok(g) = eval.recv::<Gesture>().await {
            on(g);
        }
    }
}
