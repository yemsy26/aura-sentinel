"""ver_pantalla: captura la interfaz del proyecto (página web en Edge sin ventana o juego pygame sin ventana) y, si el
modelo cargado acepta imágenes, le pide que diga si se ve bien. Las comprobaciones automáticas (texto cortado, elementos
fuera de pantalla, texto sobre fondo cargado, pantalla en negro o de un solo color) se hacen siempre."""
import base64
import json
import os
import re
import threading
import urllib.parse
from functools import partial
from http.server import ThreadingHTTPServer
from pathlib import Path

from . import app_tauri, config, llm
from .verificacion import SONDA_PYGAME, _canal_navegador, _cola, _correr, _entrada_python, _rel, _Silencioso, archivos

LOCALES = {"localhost", "127.0.0.1", "::1"}
# Probado con Qwen3.6 (Q3 + mmproj F16) en capturas reales, 3-4 muestras: una lista de comprobación con razonamiento
# no juzga mejor (o deja pasar el título tapado por figuras 3D o marca como fallo el arte 3D que sale por el borde)
# y tarda 20-35 s en vez de 6-7 s. Se queda el prompt directo.
PROMPT = ("Eres un revisor de interfaces. La imagen es una captura de {que}. Di si la interfaz se ve bien. Busca: texto "
          "cortado, solapado o ilegible; elementos fuera de pantalla o recortados; pantalla en negro o vacía; elementos "
          "descolocados o sin estilo.{extra}\nResponde en español. Primera línea exactamente 'VISUAL: OK' o "
          "'VISUAL: PROBLEMAS'; después, como mucho 6 viñetas concretas (qué falla y dónde).")

JS_PROBLEMAS = """() => {
  const W = innerWidth, out = [];
  const nombre = e => e.tagName.toLowerCase() + (e.id ? '#' + e.id : '');
  const ancho = (document.scrollingElement || document.documentElement).scrollWidth - W;
  if (ancho > 2) out.push(`la página es ${ancho}px más ancha que la ventana (scroll horizontal)`);
  for (const e of document.body.querySelectorAll('*')) {
    if (out.length >= 8) break;
    const r = e.getBoundingClientRect(), s = getComputedStyle(e), txt = e.children.length ? '' : e.textContent.trim();
    if (!r.width || !r.height || s.visibility === 'hidden' || !txt) continue;
    const parcial = (r.left < -2 && r.right > 2) || (r.left < W - 2 && r.right > W + 2);
    if ((r.right > W + 2 || r.left < -2) && (ancho > 2 || parcial))  // del todo fuera y sin scroll = panel cerrado
      out.push(`${nombre(e)} "${txt.slice(0, 40)}" se sale de la pantalla (x ${Math.round(r.left)}..${Math.round(r.right)} de ${W})`);
    else if (/hidden|clip/.test(s.overflow + s.overflowX + s.overflowY) &&
             (e.scrollWidth > e.clientWidth + 1 || e.scrollHeight > e.clientHeight + 1))
      out.push(`texto cortado en ${nombre(e)}: "${txt.slice(0, 40)}"`);
  }
  if (!document.body.innerText.trim() && !document.querySelector('canvas,img,svg,video'))
    out.push('la página está en blanco (sin texto ni imágenes)');
  return out;
}"""
JS_PIXELES = """async (b64) => {
  const img = new Image(); img.src = 'data:image/png;base64,' + b64; await img.decode();
  const c = document.createElement('canvas'); c.width = 32; c.height = 24;
  const x = c.getContext('2d'); x.imageSmoothingQuality = 'high'; x.drawImage(img, 0, 0, 32, 24);
  const d = x.getImageData(0, 0, 32, 24).data, s = new Set(); let suma = 0;
  for (let i = 0; i < d.length; i += 4) { s.add(d[i] + ',' + d[i + 1] + ',' + d[i + 2]); suma += d[i] + d[i + 1] + d[i + 2]; }
  return {colores: s.size, brillo: Math.round(suma / (d.length / 4 * 3))};
}"""
# Texto sobre fondo cargado (sin modelo). Se congela la página (requestAnimationFrame en cola, animaciones CSS y vídeos
# en pausa) y se hacen 3 capturas del mismo instante: normal, texto magenta (dónde caen las letras) y texto transparente
# (lo que hay detrás). Por palabra se mide qué parte de sus letras se confunde con lo que tiene detrás, cuántos bordes
# (líneas, aristas) hay detrás de ellas y qué parte de lo de detrás se aparta del color mayoritario (una figura).
JS_CONGELAR = """(congelar) => {
  const w = window;
  if (congelar) {
    if (!w.__sondaRAF) { w.__sondaRAF = w.requestAnimationFrame; w.__sondaCola = []; }
    w.requestAnimationFrame = cb => (w.__sondaCola.push(cb), 0);
    w.__sondaAnim = document.getAnimations().filter(a => a.playState === 'running');
    w.__sondaVid = [...document.querySelectorAll('video')].filter(v => !v.paused);
    w.__sondaAnim.forEach(a => a.pause());
    w.__sondaVid.forEach(v => v.pause());
    return new Promise(r => setTimeout(r, 120));  // que termine y se pinte el fotograma en curso
  }
  w.requestAnimationFrame = w.__sondaRAF;
  w.__sondaCola.splice(0).forEach(cb => w.__sondaRAF.call(w, cb));
  w.__sondaAnim.forEach(a => a.play());
  w.__sondaVid.forEach(v => v.play().catch(() => {}));
}"""
JS_TEXTOS = """() => {
  const W = innerWidth, H = innerHeight, vistos = new Set(), out = [];
  const caja = r => [Math.max(0, Math.floor(r.left)), Math.max(0, Math.floor(r.top)),
                     Math.min(W, Math.ceil(r.right)), Math.min(H, Math.ceil(r.bottom))];
  const tenue = e => {  // texto decorativo casi transparente: no es un fallo que se confunda con el fondo
    const s = getComputedStyle(e), c = /text/.test(s.webkitBackgroundClip + s.backgroundClip) ? '' : s.webkitTextFillColor;
    let a = +((c.match(/[\\d.]+/g) || [])[3] ?? 1);
    for (let p = e; p; p = p.parentElement) a *= +getComputedStyle(p).opacity;
    return a < 0.6 || e.closest('[aria-hidden=true]');
  };
  for (const e of document.body.querySelectorAll('*')) {
    if (out.length >= 30) break;
    if (![...e.childNodes].some(n => n.nodeType === 3 && /[\\p{L}\\p{N}]/u.test(n.textContent))) continue;
    const duenio = e.closest('h1,h2,h3,button,a,[role=button],summary') || (parseFloat(getComputedStyle(e).fontSize) >= 20 ? e : null);
    if (!duenio || vistos.has(duenio) || tenue(e)) continue;
    vistos.add(duenio);
    const palabras = [], debajo = new Set(), rec = document.createTreeWalker(duenio, NodeFilter.SHOW_TEXT);
    for (let n; (n = rec.nextNode()) && palabras.length < 40;)
      for (const m of n.textContent.matchAll(/\\S+/g)) {
        if (!/[\\p{L}\\p{N}]/u.test(m[0])) continue;
        const rg = document.createRange(); rg.setStart(n, m.index); rg.setEnd(n, m.index + m[0].length);
        const c = caja(rg.getBoundingClientRect());
        if (c[2] - c[0] < 4 || c[3] - c[1] < 4) continue;
        palabras.push({t: m[0], caja: c});
        const pila = document.elementsFromPoint((c[0] + c[2]) / 2, (c[1] + c[3]) / 2);
        for (const q of pila.slice(pila.findIndex(q => duenio.contains(q)) + 1))
          if (/^(canvas|img|video|svg)$/i.test(q.tagName)) debajo.add(q.tagName.toLowerCase());
      }
    if (palabras.length) out.push({texto: duenio.textContent.trim().replace(/\\s+/g, ' ').slice(0, 60), debajo: [...debajo], palabras});
  }
  return out;
}"""
JS_TINTAR = """(modo) => {
  let st = document.getElementById('__sonda_txt');
  if (!st) { st = document.createElement('style'); st.id = '__sonda_txt'; document.head.appendChild(st); }
  const c = modo === 'magenta' ? '#f0f' : 'transparent';
  st.textContent = modo ? `*, *::before, *::after, ::placeholder { color: ${c} !important; -webkit-text-fill-color: ${c} !important;
    text-shadow: none !important; text-decoration-color: ${c} !important; transition: none !important; }` : '';
  for (const e of document.body.querySelectorAll('*')) {
    const s = getComputedStyle(e), en = e.style, previo = e.dataset.sondaClip;
    if (modo && /text/.test(s.webkitBackgroundClip + s.backgroundClip)) {  // texto con degradado: el fondo es la letra
      if (previo === undefined)
        e.dataset.sondaClip = en.getPropertyPriority('background-image') + '|' + en.getPropertyValue('background-image');
      en.setProperty('background-image', 'none', 'important');
    } else if (!modo && previo !== undefined) {  // deja el estilo en línea como estaba
      const i = previo.indexOf('|'), valor = previo.slice(i + 1);
      if (valor) en.setProperty('background-image', valor, previo.slice(0, i)); else en.removeProperty('background-image');
      delete e.dataset.sondaClip;
    }
  }
  return new Promise(r => setTimeout(r, 50));
}"""
JS_FONDO = """async ([imgs, textos]) => {
  const leer = async s => { const i = new Image(); i.src = 'data:image/png;base64,' + s; await i.decode();
    const c = document.createElement('canvas'); c.width = i.width; c.height = i.height;
    const x = c.getContext('2d'); x.drawImage(i, 0, 0); return x.getImageData(0, 0, i.width, i.height); };
  const datos = await Promise.all(imgs.map(s => s && leer(s))), W = datos[0].width;
  const [A, M, B, B2] = datos.map(d => d && d.data);
  const Y = (d, i) => 0.299 * d[i] + 0.587 * d[i + 1] + 0.114 * d[i + 2];
  const lin = v => (v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  const L = (d, i) => 0.2126 * lin(d[i]) + 0.7152 * lin(d[i + 1]) + 0.0722 * lin(d[i + 2]);
  const pct = (v, p) => v.length ? v[Math.floor(p * (v.length - 1))] : 0;
  const medir = ([x0, y0, x1, y1]) => {
    const w = x1 - x0, h = y1 - y0, alfa = new Float32Array(w * h), fila = new Uint8Array(w * h), zona = new Uint8Array(w * h);
    const px = k => ((y0 + (k / w | 0)) * W + x0 + k % w) * 4;
    let movido = 0;
    for (let k = 0; k < w * h; k++) {  // cobertura de la letra: M = alfa * magenta + (1 - alfa) * B
      const i = px(k); let den = 0, a = 0;
      for (const [c, v] of [[0, 255], [1, 0], [2, 255]])
        if (Math.abs(v - B[i + c]) > Math.abs(den)) { den = v - B[i + c]; a = (M[i + c] - B[i + c]) / den; }
      alfa[k] = Math.abs(den) < 60 ? 0 : Math.max(0, Math.min(1, a));
      if (B2 && Math.abs(B[i] - B2[i]) + Math.abs(B[i + 1] - B2[i + 1]) + Math.abs(B[i + 2] - B2[i + 2]) > 30) movido++;
    }
    for (let k = 0; k < w * h; k++) { const x = k % w;  // zona = letras + 2 px alrededor
      for (let d = -2; d <= 2; d++) if (x + d >= 0 && x + d < w && alfa[k + d] > 0.3) { fila[k] = 1; break; } }
    for (let k = 0; k < w * h; k++)
      for (let d = -2; d <= 2; d++) { const j = k + d * w; if (j >= 0 && j < w * h && fila[j]) { zona[k] = 1; break; } }
    const yt = [], yf = [], yz = [];  // luminancia de la letra, de lo que la rodea y de todo lo que hay detrás
    for (let k = 0; k < w * h; k++) {
      if (!zona[k]) continue;
      const i = px(k); yz.push(Y(B, i));
      if (alfa[k] > 0.7) yt.push(Y(A, i)); else yf.push(Y(B, i));
    }
    [yt, yf, yz].forEach(v => v.sort((a, b) => a - b));
    const delta = Math.max(40, Math.abs(pct(yt, 0.5) - pct(yf, 0.5))), umbral = Math.max(24, 0.3 * delta);
    const base = [0, 1, 2].map(c => { const v = []; for (let k = 0; k < w * h; k++) if (zona[k]) v.push(B[px(k) + c]);
                                      return pct(v.sort((a, b) => a - b), 0.5); });  // color mayoritario detrás
    let nucleo = 0, mezcla = 0, n = 0, bordes = 0, cubierta = 0;
    for (let k = 0; k < w * h; k++) {
      const i = px(k);
      if (alfa[k] > 0.7) { nucleo++; const la = L(A, i), lb = L(B, i);
        if ((Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05) < 1.8) mezcla++; }  // la letra no destaca de lo de detrás
      if (!zona[k] || k % w === w - 1 || k >= w * (h - 1)) continue;
      n++; const yb = Y(B, i);
      if (Math.abs(Y(B, i + 4) - yb) + Math.abs(Y(B, i + W * 4) - yb) > umbral) bordes++;
      if (Math.hypot(B[i] - base[0], B[i + 1] - base[1], B[i + 2] - base[2]) > Math.max(40, 0.35 * delta)) cubierta++;
    }
    return {nucleo, mezcla: nucleo ? mezcla / nucleo : 0, bordes: n ? bordes / n : 0, cubierta: n ? cubierta / n : 0,
            rango: Math.round(pct(yz, 0.95) - pct(yz, 0.05)), movido: movido / (w * h)};
  };
  return textos.map(t => ({...t, palabras: t.palabras.map(p => ({t: p.t, ...medir(p.caja)}))}));
}"""
# Palabra mal: >= 20 % de sus letras no destaca de lo que tiene detrás, >= 15 % de bordes detrás o, con un canvas o svg
# detrás (figuras dibujadas; una foto o un vídeo de fondo suelen ser a propósito), >= 25 % de lo de detrás distinto del
# color mayoritario: una figura tapa parte de la palabra.
CONFUNDIDA, CARGADA, CUBIERTA = 0.2, 0.15, 0.25
INSTANTES = 4  # con canvas o vídeo detrás del texto la escena se mueve: se mira en varios instantes (cada 0,6 s)


def _pixeles(d: dict) -> list[str]:
    """d = {colores, brillo} de la captura reducida a 32x24."""
    if d["colores"] <= 2 and d["brillo"] < 12:
        return ["la pantalla está en NEGRO (o casi): no se ve nada dibujado"]
    if d["colores"] == 1:
        return ["la pantalla es de un solo color: no se ve nada dibujado"]
    return []


def _juzgar_fondo(instantes: list[list[dict]]) -> tuple[list[str], int]:
    """instantes: lo que mide JS_FONDO en cada instante. Devuelve (problemas, índice del instante con más palabras mal).
    No cuenta palabras con pocas letras visibles, sobre un fondo liso (eso sería poco contraste, no fondo cargado) ni
    con el fondo aún moviéndose entre dos capturas del mismo instante (no se pudo congelar)."""
    malos, peso = {}, [0] * len(instantes)
    for k, textos in enumerate(instantes):
        for t in textos:
            ok = [p for p in t["palabras"] if p["nucleo"] >= 40 and p["rango"] >= 20 and p["movido"] <= 0.02]
            conf = [p["t"] for p in ok if p["mezcla"] >= CONFUNDIDA]
            carg = [p["t"] for p in ok if p["bordes"] >= CARGADA]
            tap = [p["t"] for p in ok if p["cubierta"] >= CUBIERTA] if {"canvas", "svg"} & set(t["debajo"]) else []
            if not conf and not carg and not tap:
                continue
            m = malos.setdefault(t["texto"], {"debajo": t["debajo"], "veces": 0, "conf": [], "carg": [], "tap": []})
            m["veces"] += 1
            for clave, lista in (("conf", conf), ("carg", carg), ("tap", tap)):
                m[clave] += [x for x in lista if x not in m[clave]]
            peso[k] += len(conf) + len(carg) + len(tap)
    nombres = {"canvas": "canvas", "img": "imagen", "video": "vídeo", "svg": "svg"}
    problemas = []
    for texto, m in list(malos.items())[:4]:
        partes = [" y ".join(nombres[d] for d in m["debajo"]) + " detrás"] if m["debajo"] else []
        tap = [x for x in m["tap"] if x not in m["conf"] + m["carg"]]
        for lista, que in ((m["conf"], "se confunde con el fondo: "), (m["carg"], "líneas/bordes detrás de "),
                           (tap, "figura detrás de ")):
            if lista:
                partes.append(que + ", ".join(f"«{x}»" for x in lista[:4]))
        if len(instantes) > 1:
            partes.append(f"en {m['veces']} de {len(instantes)} instantes: la escena se mueve")
        problemas.append(f"texto sobre fondo cargado: «{texto}» ({'; '.join(partes)})")
    return problemas, max(range(len(peso)), key=peso.__getitem__)


def _fondo_cargado(pag, aux) -> tuple[bytes, list[str]]:
    """Captura la página congelada y busca texto sobre fondo cargado. Si hay canvas o vídeo detrás de algún texto,
    repite en varios instantes. Devuelve (captura del peor instante o, si todo está bien, del primero; problemas)."""
    capturas, instantes, n = [], [], 1
    while len(instantes) < n:
        if instantes:
            pag.wait_for_timeout(600)
        pag.evaluate(JS_CONGELAR, True)
        a = pag.screenshot()
        if capturas and a == capturas[-1]:  # nada se mueve: más instantes darían lo mismo
            pag.evaluate(JS_CONGELAR, False)
            break
        textos = pag.evaluate(JS_TEXTOS)
        pag.evaluate(JS_TINTAR, "magenta")
        m = pag.screenshot()
        pag.evaluate(JS_TINTAR, "transparente")
        b = pag.screenshot()
        pag.wait_for_timeout(100)
        b2 = pag.screenshot()
        pag.evaluate(JS_TINTAR, "")
        pag.evaluate(JS_CONGELAR, False)
        imgs = [base64.b64encode(x).decode() for x in (a, m, b)] + [None if b2 == b else base64.b64encode(b2).decode()]
        capturas.append(a)
        instantes.append(aux.evaluate(JS_FONDO, [imgs, textos]) if textos else [])
        if any({"canvas", "video"} & set(t["debajo"]) for t in textos):
            n = INSTANTES
    problemas, peor = _juzgar_fondo(instantes)
    return capturas[peor], problemas


def capturar_web(raiz: Path, objetivo: str, png: Path, espera: float) -> list[str]:
    """objetivo: URL local o ruta relativa de un .html (se sirve la carpeta del proyecto). Devuelve los problemas."""
    from playwright.sync_api import sync_playwright
    servidor = None
    if not objetivo.startswith(("http://", "https://")):
        servidor = ThreadingHTTPServer(("127.0.0.1", 0), partial(_Silencioso, directory=str(raiz)))
        threading.Thread(target=servidor.serve_forever, daemon=True).start()
        objetivo = f"http://127.0.0.1:{servidor.server_port}/{urllib.parse.quote(objetivo.replace(chr(92), '/'))}"
    try:
        with sync_playwright() as p:
            nav = _canal_navegador(p)
            if nav is None:
                raise RuntimeError("no se pudo abrir Edge/Chrome sin ventana")
            try:
                pag = nav.new_page(viewport={"width": 1024, "height": 768})
                errores = []
                pag.on("pageerror", lambda e: errores.append(f"excepción JS: {str(e)[:160]}"))
                pag.goto(objetivo, wait_until="load", timeout=20000)
                pag.wait_for_timeout(int(espera * 1000))
                aux = nav.new_page()
                datos, fondo = _fondo_cargado(pag, aux)
                png.write_bytes(datos)
                problemas = pag.evaluate(JS_PROBLEMAS) + fondo + errores[:2]
                return problemas + _pixeles(aux.evaluate(JS_PIXELES, base64.b64encode(datos).decode()))
            finally:
                nav.close()
    finally:
        if servidor:
            servidor.shutdown()
            servidor.server_close()


def capturar_pygame(entrada: Path, python: str, png: Path, espera: float) -> list[str]:
    """Juega sin ventana con la sonda (teclas aleatorias, reloj acelerado) y guarda la última pantalla."""
    env = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8", SDL_VIDEODRIVER="dummy", SDL_AUDIODRIVER="dummy",
               SONDA_CAPTURA=str(png), SONDA_SEMILLA="1")
    codigo, salida = _correr([python, str(SONDA_PYGAME), str(entrada), str(espera)], entrada.parent, espera + 25, env)
    m = re.search(r"^SONDA (\{.*\})$", salida, re.MULTILINE)
    if not m:
        if "No module named 'pygame'" in salida:
            raise RuntimeError("pygame no está instalado en el entorno del proyecto (.venv)")
        raise RuntimeError("el juego no se cierra al recibir QUIT" if codigo is None else f"el juego no arrancó:\n    {_cola(salida)}")
    d = json.loads(m.group(1))
    cap = d.get("captura") or {}
    if "colores" not in cap:
        raise RuntimeError(cap.get("error") or "el juego no creó ninguna ventana (pygame.display.set_mode)")
    problemas = _pixeles(cap)
    for t in d.get("textos_cortados") or []:
        problemas.append(f"texto cortado por el borde {t['borde']}: '{t['texto']}' en x={t['x']}, y={t['y']} "
                         f"({t['ancho']}x{t['alto']} px; pantalla {cap['ancho']}x{cap['alto']}; cortado en {t['veces']} "
                         f"de {t['de']} dibujados)")
    if d["error"]:
        problemas.append("el juego lanzó una excepción: " + d["error"].strip().splitlines()[-1][:200])
    return problemas


def modelo_vision(cfg: dict | None = None) -> tuple[str | None, str | None, str]:
    """(servidor, modelo, motivo). Si el modelo cargado no ve imágenes: servidor=None y el motivo."""
    try:
        base, modelo, _ = llm.detectar(cfg or config.cargar())
        if not modelo:
            return None, None, "no hay ningún modelo cargado"
        info = next((m for m in llm.listar_modelos(base)
                     if any(i.get("id") == modelo for i in m.get("loaded_instances") or [])), {})
    except llm.ErrorLLM as e:
        return None, None, f"sin conexión con el modelo ({e})"
    if (info.get("capabilities") or {}).get("vision") is False:
        return None, modelo, f"el modelo cargado ({modelo}) no acepta imágenes"
    return base, modelo, ""


def evaluar(png: bytes, que: str, pregunta: str = "", cfg: dict | None = None) -> tuple[str | None, str]:
    """(respuesta del modelo o None, de dónde sale el juicio). Nunca lanza: sin visión lo dice."""
    base, modelo, motivo = modelo_vision(cfg)
    if not base:
        return None, f"sin visión: {motivo}"
    extra = f" Además: {pregunta.strip()}" if pregunta.strip() else ""
    cuerpo = {"model": modelo, "max_tokens": 700, "temperature": 0.2, "reasoning_effort": "none", "messages": [
        {"role": "user", "content": [
            {"type": "text", "text": PROMPT.format(que=que, extra=extra)},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64," + base64.b64encode(png).decode()}}]}]}
    try:
        r = llm.http_json(f"{base}/v1/chat/completions", cuerpo, timeout=300)
    except llm.ErrorLLM as e:
        if re.search(r"image|imagen|vision", str(e), re.IGNORECASE):
            return None, f"sin visión: el modelo cargado ({modelo}) no acepta imágenes"
        return None, f"sin visión: falló la consulta al modelo: {str(e)[:200]}"
    texto = (((r.get("choices") or [{}])[0].get("message") or {}).get("content") or "")
    texto = re.sub(r"<think>.*?</think>", "", texto, flags=re.DOTALL).strip()
    return (texto, f"revisado por {modelo}") if texto else (None, f"sin visión: {modelo} no respondió")


def _elegir(raiz: Path) -> str:
    """Sin ruta: la página principal o, si es un juego pygame, su archivo de entrada."""
    htmls = sorted(archivos(raiz, ".html", ".htm"), key=lambda p: (p.name != "index.html", len(p.parts)))
    if htmls:
        return _rel(htmls[0], raiz)
    pys = archivos(raiz, ".py")
    if any("pygame" in p.read_text(encoding="utf-8", errors="replace") for p in pys if not p.name.startswith("test")):
        entrada = _entrada_python(raiz, pys)
        return _rel(entrada, raiz) if entrada else ""
    return ""


def _tauri(raiz: Path) -> Path | None:
    """La carpeta con tauri.conf.json (src-tauri) si el proyecto es una app Tauri."""
    from .verificacion import ignorado
    return next((p.parent for p in sorted(raiz.rglob("tauri.conf.json")) if not ignorado(p, raiz)), None)


def _es_dev(objetivo: str, d: Path) -> bool:
    """La URL es el devUrl de la app: en un navegador se ve el frontend sin Tauri (sin invoke ni datos)."""
    try:
        dev = (json.loads((d / "tauri.conf.json").read_text(encoding="utf-8")).get("build") or {}).get("devUrl") or ""
    except (OSError, ValueError, AttributeError):
        return False
    url, suya = urllib.parse.urlparse(objetivo), urllib.parse.urlparse(str(dev))
    return bool(suya.port) and url.hostname in LOCALES and url.port == suya.port


def ver_app(h, d: Path, png: Path, pregunta: str, espera: float) -> tuple[str, str]:
    """(informe de la ventana real de la app Tauri, o '' y el motivo si no se pudo abrir)."""
    from .ecosistemas import ENTORNO
    r = app_tauri.probar(d, ENTORNO, png, espera)
    if r["estado"] == "omitido":
        return "", r["motivo"]
    juicio, fuente = (evaluar(png.read_bytes(), "la ventana de la app de escritorio (Tauri)", pregunta,
                              getattr(h, "cfg_llm", None)) if png.is_file() else (None, "sin captura"))
    mal = bool(r["errores"]) or bool(juicio and re.search(r"VISUAL:\s*PROBLEMAS", juicio, re.IGNORECASE))
    partes = [f"VISUAL: {'PROBLEMAS' if mal else 'OK'} en la app Tauri (su ventana de verdad, no un navegador; {fuente})",
              f"Captura: {h.rel(png)}" if png.is_file() else "", f"Texto de la ventana: «{r['texto'] or '(ninguno)'}»"]
    if r["errores"]:
        partes.append("Errores (consola o la app):\n" + "\n".join(f"- {e}" for e in r["errores"]))
    if r["consola"]:
        partes.append("Consola: " + " | ".join(r["consola"]))
    if juicio:
        detalle = re.sub(r"^\s*VISUAL:\s*\w+\s*|\s*VISUAL:\s*\w+\s*$", "", juicio).strip() or juicio
        partes.append(detalle[:1800])
    return "\n".join(p for p in partes if p), ""


def ver(h, ruta: str = "", pregunta: str = "", segundos: float = 2) -> str:
    espera = max(0.5, min(float(segundos or 2), 20))
    objetivo = (ruta or "").strip().strip('"')
    nota, d = "", _tauri(h.raiz)
    if d and (not objetivo or objetivo.lower() in ("app", "tauri") or _es_dev(objetivo, d)):
        (h.dir_agente / "capturas").mkdir(parents=True, exist_ok=True)
        informe, motivo = ver_app(h, d, h.dir_agente / "capturas" / "app_tauri.png", pregunta, max(espera, 3))
        if informe:
            return informe
        nota = (f"\nNOTA: no pude abrir la app de verdad ({motivo}). Esto es el frontend en un navegador normal, FUERA "
                "de Tauri: aquí invoke no existe, así que si faltan los datos NO es un fallo de la app. Para ver la app, "
                "deja libre ese puerto y llama a ver_pantalla sin ruta.")
        objetivo = "" if objetivo.lower() in ("app", "tauri") else objetivo
    objetivo = objetivo or _elegir(h.raiz)
    if not objetivo:
        return "No encontré qué mostrar: indica ruta (un .html, el .py del juego pygame o http://localhost:PUERTO)."
    if objetivo.startswith(("http://", "https://")):
        if urllib.parse.urlparse(objetivo).hostname not in LOCALES:
            return "Solo se capturan páginas locales (http://localhost:PUERTO/...). Para webs externas usa web_leer."
        tipo, que, nombre = "web", objetivo, urllib.parse.urlparse(objetivo).port or "web"
    else:
        p = h.ruta(objetivo)
        if not p.is_file():
            return f"No existe el archivo: {objetivo}. Usa listar para ver los nombres reales."
        tipo = {".py": "pygame", ".html": "web", ".htm": "web"}.get(p.suffix.lower())
        if not tipo:
            return "ver_pantalla acepta un .html, el .py de un juego pygame o una URL http://localhost:PUERTO."
        que, nombre = h.rel(p), p.stem
    carpeta = h.dir_agente / "capturas"
    carpeta.mkdir(parents=True, exist_ok=True)
    png = carpeta / f"pantalla_{nombre}.png"
    try:
        problemas = (capturar_web(h.raiz, que, png, espera) if tipo == "web"
                     else capturar_pygame(p, h.python_proyecto(), png, espera))
    except ImportError:
        return "ERROR: falta playwright en el agente (ejecuta instalar.bat)."
    except Exception as e:
        return f"ERROR: no se pudo capturar {que}: {e}"
    juicio, fuente = evaluar(png.read_bytes(), f"{'la página web' if tipo == 'web' else 'el juego pygame'} {que}",
                             pregunta, getattr(h, "cfg_llm", None))
    mal = bool(problemas) or bool(juicio and re.search(r"VISUAL:\s*PROBLEMAS", juicio, re.IGNORECASE))
    partes = [f"VISUAL: {'PROBLEMAS' if mal else 'OK'} en {que} ({fuente})", f"Captura: {h.rel(png)}"]
    if problemas:
        partes.append("Comprobación automática:\n" + "\n".join(f"- {x}" for x in problemas))
    if juicio:
        detalle = re.sub(r"^\s*VISUAL:\s*\w+\s*|\s*VISUAL:\s*\w+\s*$", "", juicio).strip() or juicio
        partes.append(detalle if len(detalle) <= 1800 else detalle[:1800] + "\n[... juicio recortado]")
    else:
        partes.append("Solo comprobaciones automáticas (texto cortado o sobre fondo cargado, fuera de pantalla, pantalla en "
                      "negro). Para el juicio visual hace falta un modelo con visión cargado (o su mmproj).")
    return "\n".join(partes) + nota
