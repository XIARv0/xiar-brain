// SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
import { useCallback, useEffect, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import * as THREE from 'three'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'
import { mergeVertices } from 'three/examples/jsm/utils/BufferGeometryUtils.js'
import { command } from './invoke'
import './swipe-deck.css'
import { motionFrame, cancelMotionFrame, motionNow } from '../../../os/xiar-desktop/src/lib/desktopState'   // t192u: the Animation switch (Settings > Screen)

/**
 * Swipe Deck (Menu > Panels > Swipe Deck, route /panels/swipe-deck). A full-screen page (no holo panel, no holo table), laid out like Galaxy's Swipe Deck:
 * the score, the daily quests and the judges on the left, the card to swipe on the right, and one glass brain in the holo table's place, with the 99 mental models as dots inside it, a cluster per family.
 * The mental-model memory improves by being judged. Three kinds of card:
 *   Laya's pick   the models Laya offered the agent for a task;
 *   Agent's pick  the one model the agent actually opened to decide with;
 *   Outcome       the decision it made with that model.
 * Swipe a card right for a heart (good) or left for a skull (bad); the heart and skull buttons and the arrow keys do the same. Points: you 3, the cloud bot 2, the bigger local bot 2,
 * the agent 1 (its own choice), Laya 1 (its own pick). The bots judge BLIND and get each card at the same time as you (t134u); their opinions stay sealed until you vote. You may leave the voting to the bots (Judges).
 * A card stays in the deck until every switched-on judge has voted, then it moves to the history. Hearts and skulls rank the models Laya picks from.
 * Layers, back to front: the menu's clouds (the page backdrop), the glass brain (the one from the VR XIAR memory graph, public/models/glass-brain.glb; it turns slowly and floats like the table did; each family is a named cluster of dots inside it; green glow = loved, red = skulled, bigger = more votes; click a dot to read the model),
 * then the deck. Click any model on a card or in the history to read its note.
 */
type Model = { n: number; name: string; group: string; when: string }
type Rank = { hearts: number; skulls: number; weight: number; factor: number }
type Card = { id: string; agent: string; kind: 'suggest' | 'use' | 'outcome'; task: string; answer?: string; models: number[]; names: string[]; scores: number[]; sealed: string[] }
type Done = Omit<Card, 'sealed'> & { verdicts: Record<string, number> }
type Quest = { id: string; label: string; have: number; need: number; done: boolean }
type Stats = { hearts: number; skulls: number; combo: number; agreements: number; xp: number; level: number; cards_left: number; waiting_for_bots: number; finished: number; bot_votes: number
  quests: Quest[]; streak: number; best_streak: number; bonus_xp: number; quest_xp: number }
type Settings = { owner: boolean; cloud: string | null; local: string | null }
type State = { stats: Stats; card: Card | null; history: Done[]; settings: Settings; ranks: Record<string, Rank>; models: Model[] }
type Judges = { settings: Settings; local_now: string | null; connections: { id: string; where: string }[] }
type Reveal = { reveal: Record<string, string>; agreed: string[] }
type Note = { n: number; name: string; group: string; when: string; note: string }

/* The rings behind the card: a camera-lens style HUD of fine tick rings, a blue band, a scale arc with numbers and segmented arcs, each turning at its own speed. */
const arcPath = (r: number, a0: number, a1: number) => {
  const p = (a: number) => [(r * Math.cos((a * Math.PI) / 180)).toFixed(2), (r * Math.sin((a * Math.PI) / 180)).toFixed(2)]
  const [x0, y0] = p(a0), [x1, y1] = p(a1)
  return `M${x0} ${y0}A${r} ${r} 0 ${a1 - a0 > 180 ? 1 : 0} 1 ${x1} ${y1}`
}
function HudRings() {
  return (
    <svg className="sd-rings" viewBox="-100 -100 200 200" aria-hidden="true">
      <defs>
        <linearGradient id="sd-gold" gradientUnits="userSpaceOnUse" x1="-100" y1="-100" x2="100" y2="100"><stop offset="0" stopColor="#fff3c4" /><stop offset="0.22" stopColor="#e8b73a" /><stop offset="0.45" stopColor="#8a5a08" /><stop offset="0.62" stopColor="#f7d674" /><stop offset="0.82" stopColor="#b8801c" /><stop offset="1" stopColor="#ffe9a0" /></linearGradient>
        <linearGradient id="sd-pink" gradientUnits="userSpaceOnUse" x1="100" y1="-100" x2="-100" y2="100"><stop offset="0" stopColor="#ff7ac0" /><stop offset="0.5" stopColor="#ff1f8e" /><stop offset="1" stopColor="#c2006b" /></linearGradient>
      </defs>
      <g className="sd-spin sd-s1"><circle r="96" className="hud-ticks" strokeWidth="5" strokeDasharray="0.9 2.8" opacity="0.9" /><circle r="96" className="hud-ticks hud-alt" strokeWidth="5" strokeDasharray="0.9 2.8" strokeDashoffset="-1.85" opacity="0.9" /></g>
      <g className="sd-spin sd-s2">
        <path d={arcPath(89, 200, 345)} className="hud-line" /><path d={arcPath(86, 205, 340)} className="hud-ticks" strokeWidth="3" strokeDasharray="0.5 2.2" />
        <path d="M-70 -57L-62 -57L-66 -63Z" className="hud-fill" transform="translate(30 -14)" />
      </g>
      <circle r="81" className="hud-line" opacity="0.6" />
      <g className="sd-spin sd-s3"><circle r="75" className="hud-band" strokeWidth="11" /><circle r="70" className="hud-ticks" strokeWidth="2.2" strokeDasharray="0.6 3.4" opacity="0.85" /><circle r="70" className="hud-ticks hud-alt" strokeWidth="2.2" strokeDasharray="0.6 3.4" strokeDashoffset="-2" opacity="0.85" /></g>
      <circle r="79" className="hud-line" strokeDasharray="1.2 1.4" opacity="0.7" />
      <g className="sd-spin sd-s4">
        <path d={arcPath(84, -25, 70)} className="hud-line" />
        <path d={arcPath(80, -25, 70)} className="hud-ticks" strokeWidth="4" strokeDasharray="0.45 3.4" />
        {[0, 1, 2, 3, 4, 5].map((i) => <rect key={i} x={86 + i * 0} y={-2} width={4 + (i % 3) * 3} height="1.6" className="hud-bar" transform={`rotate(${-12 + i * 15} 0 0) translate(${i % 2 ? 2 : 0} 0)`} />)}
        {['100', '200', '400', '600', '800'].map((t, i) => <text key={t} className="hud-num" transform={`rotate(${-10 + i * 17}) translate(93 0) rotate(90)`}>{t}</text>)}
      </g>
      <circle r="62" className="hud-line" strokeDasharray="2.4 1.2" opacity="0.8" />
      <g className="sd-spin sd-s5">
        <path d={arcPath(50, 20, 250)} className="hud-seg hud-gold" strokeWidth="6" strokeDasharray="3.2 5.2" />
        <path d={arcPath(50, 20, 250)} className="hud-seg" strokeWidth="6" strokeDasharray="3.2 5.2" strokeDashoffset="-4.2" />
        <path d={arcPath(42, 70, 230)} className="hud-seg hud-gold" strokeWidth="5" strokeDasharray="2.6 4.2" />
        <path d={arcPath(42, 70, 230)} className="hud-seg" strokeWidth="5" strokeDasharray="2.6 4.2" strokeDashoffset="-3.4" />
      </g>
      <circle r="56" className="hud-line" opacity="0.9" /><circle r="34" className="hud-line hud-glow" />
      <g className="sd-spin sd-s6"><path d={arcPath(28, 0, 120)} className="hud-seg" strokeWidth="1.6" /><path d={arcPath(28, 180, 300)} className="hud-seg" strokeWidth="1.6" /></g>
    </svg>
  )
}

const BOT: Record<string, string> = { cloud: 'Cloud bot', local: 'Bigger local bot' }
const WHO: Record<string, string> = { owner: 'You', cloud: 'Cloud', local: 'Local', agent: 'Agent', laya: 'Laya', auto: 'Auto' }
const KIND: Record<string, string> = { suggest: "Laya's pick", use: "Agent's pick", outcome: 'Outcome' }
const GROUP_COLOURS = ['#f5c542', '#4ad0ff', '#a07bff', '#ff6aa8', '#5be0a6', '#ff9a4a', '#8fa8ff', '#ffd1e8', '#7ee3ff']
const SWIPE_AT = 110   // pixels of drag that count as a swipe

export default function SwipeDeckView() {
  const [state, setState] = useState<State | null>(null)
  const [error, setError] = useState('')
  const [revealed, setRevealed] = useState('')
  const [judges, setJudges] = useState<Judges | null>(null)
  const [asking, setAsking] = useState(false)
  const [flash, setFlash] = useState<'heart' | 'skull' | null>(null)
  const [hover, setHover] = useState('')
  const tip = useRef<HTMLDivElement>(null)
  const [note, setNote] = useState<Note | null>(null)
  const [dx, setDx] = useState(0)
  const mount = useRef<HTMLDivElement>(null)
  const scene = useRef<{ update: (s: State) => void } | null>(null)
  const openNote = useRef<(n: number) => void>(() => undefined)
  const drag = useRef<{ x: number } | null>(null)

  const load = useCallback(async () => {
    try { setState(await command<State>('swipe_state')); setError('') } catch (e) { setError((e as Error).message) }
  }, [])
  const loadJudges = useCallback(async () => { try { setJudges(await command<Judges>('swipe_judges')) } catch { /* judges are optional */ } }, [])
  useEffect(() => { void load(); void loadJudges(); const t = window.setInterval(load, 12000); return () => window.clearInterval(t) }, [load, loadJudges])

  openNote.current = async (n: number) => { try { setNote(await command<Note>('swipe_note', { n })) } catch (e) { setError((e as Error).message) } }

  const vote = useCallback(async (value: 'heart' | 'skull') => {
    if (!state?.card) return
    setFlash(value); window.setTimeout(() => setFlash(null), 450)
    try {
      const r = await command<Reveal>('swipe_vote', { pick: state.card.id, value })   // the bots' sealed opinions come back only now
      const said = Object.entries(r.reveal).map(([k, v]) => `${BOT[k] ?? k} gave a ${v}${r.agreed.includes(k) ? ' (agrees, +5 XP)' : ''}`)
      for (const k of ['cloud', 'local'] as const) if (state?.settings?.[k] && !(k in r.reveal)) said.push(`${BOT[k]} is still judging; its verdict goes in your log`)   // cards reach you and the bots at the same time (t134u)
      setRevealed(said.join(' · ')); await load()
    } catch (e) { setError((e as Error).message) }
  }, [state, load])

  const askBots = useCallback(async () => {   // the bots that are on go through the waiting cards now (they also do it on their own, every minute)
    setAsking(true)
    try { await command('swipe_judge_now', { limit: 4 }); await load() } catch (e) { setError((e as Error).message) } finally { setAsking(false) }
  }, [load])
  const setJudge = useCallback(async (role: 'owner' | 'cloud' | 'local', value: string | boolean) => {
    try { await command('swipe_judges_set', role === 'owner' ? { role, value } : { role, connection: value }); await loadJudges(); await load() } catch (e) { setError((e as Error).message) }
  }, [load, loadJudges])

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { setNote(null); return }   // works even while the chat box has focus
      if ((e.target as HTMLElement)?.closest('input,textarea,select')) return
      if (e.key === 'ArrowRight') void vote('heart'); else if (e.key === 'ArrowLeft') void vote('skull')
    }
    window.addEventListener('keydown', key); return () => window.removeEventListener('keydown', key)
  }, [vote])

  // ---- swiping the card
  const down = (e: React.PointerEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest('button')) return
    drag.current = { x: e.clientX }; e.currentTarget.setPointerCapture(e.pointerId)
  }
  const move = (e: React.PointerEvent<HTMLDivElement>) => { if (drag.current) setDx(e.clientX - drag.current.x) }
  const up = () => {
    const d = dx; drag.current = null; setDx(0)
    if (d > SWIPE_AT) void vote('heart'); else if (d < -SWIPE_AT) void vote('skull')
  }

  // ---- the arena: ONE glass brain, slowly turning, standing where the holo table was. The 99 models are dots inside it, a cluster per family, each family named inside the glass.
  // The brain is the one from the VR XIAR memory graph (public/models/glass-brain.glb = crates/memory/assets/brain_mesh_source.glb of "VR XIAR V0 version 1"), drawn as glass like its
  // M_CelestialGlass (a bright rim, a faint body). The dots are placed the way that graph places its notes: hubs spread across the mesh's own vertices, the dots jittered around them
  // and kept inside the shape. If the model cannot be loaded, a brain built in code stands in.
  useEffect(() => {
    const host = mount.current; if (!host) return
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true }); renderer.setPixelRatio(Math.min(devicePixelRatio, 2)); host.appendChild(renderer.domElement)
    const sc = new THREE.Scene(); const cam = new THREE.PerspectiveCamera(42, 1, 0.1, 100); cam.position.set(0, 3.6, 18.5); cam.lookAt(0, 0, 0)
    const root = new THREE.Group(); sc.add(root)
    const glass = new THREE.ShaderMaterial({
      transparent: true, side: THREE.DoubleSide, depthWrite: false,
      uniforms: { uBase: { value: new THREE.Color('#3b2bd6') }, uRim: { value: new THREE.Color('#8fe9ff') } },
      vertexShader: 'varying vec3 vN; varying vec3 vV; void main(){ vN = normalize(normalMatrix * normal); vec4 mv = modelViewMatrix * vec4(position, 1.0); vV = normalize(-mv.xyz); gl_Position = projectionMatrix * mv; }',
      fragmentShader: 'uniform vec3 uBase; uniform vec3 uRim; varying vec3 vN; varying vec3 vV; void main(){ float f = pow(1.0 - abs(dot(normalize(vN), normalize(vV))), 2.4); gl_FragColor = vec4(mix(uBase, uRim, f), 0.07 + f * 0.62); }',
    })
    const brain = new THREE.Group(); root.add(brain)
    const disposables: { dispose: () => void }[] = [glass]

    // where the families' dots live: a hub per family and a way to keep a dot inside the glass
    type Shape = { hubs: THREE.Vector3[]; keep: (p: THREE.Vector3) => void }
    let shape: Shape | null = null
    const SIZE = 4.5      // the model's half-length, in scene units

    const fromMesh = (source: THREE.BufferGeometry): Shape => {
      const geo = source.clone(); geo.computeBoundingBox()
      const p = geo.attributes.position as THREE.BufferAttribute; const c = new THREE.Vector3()
      for (let i = 0; i < p.count; i++) c.add(new THREE.Vector3().fromBufferAttribute(p, i))
      c.divideScalar(p.count); geo.translate(-c.x, -c.y, -c.z); geo.computeVertexNormals()
      const mesh = new THREE.Mesh(geo, glass); mesh.scale.setScalar(SIZE); brain.add(mesh); disposables.push(geo)     // the file is already upright: y is up, x runs front to back, the two hemispheres are split along z
      const world: THREE.Vector3[] = []                  // every vertex as the scene sees it (the same scale, so the dots line up with the glass)
      for (let i = 0; i < p.count; i++) { const v = new THREE.Vector3().fromBufferAttribute(p, i); world.push(v.multiplyScalar(SIZE)) }
      // how far the glass reaches in each direction from the middle (a lat-long grid of the farthest vertex), so a dot can be kept inside
      const BU = 36, BV = 18; const reach = new Float32Array(BU * BV)
      const cell = (d: THREE.Vector3) => Math.min(BV - 1, Math.floor((d.y * 0.5 + 0.5) * BV)) * BU + (Math.floor(((Math.atan2(d.z, d.x) + Math.PI) / (2 * Math.PI)) * BU) % BU)
      world.forEach((w) => { const len = w.length(); const k = cell(w.clone().divideScalar(len)); reach[k] = Math.max(reach[k], len) })
      const known = [...reach].filter((x) => x > 0); const mean = known.reduce((a, b) => a + b, 0) / Math.max(1, known.length)
      for (let k = 0; k < reach.length; k++) if (!reach[k]) reach[k] = mean * 0.6
      const keep = (q: THREE.Vector3) => { const len = q.length(); if (len < 1e-6) return; const max = 0.8 * reach[cell(q.clone().divideScalar(len))]; if (len > max) q.multiplyScalar(max / len) }
      // hubs spread across the mesh's own vertices (farthest-point sampling, as the memory graph does), then drawn inward so the clusters sit inside the glass
      const sample = world.filter((_, i) => i % 6 === 0); const picked: THREE.Vector3[] = [sample.reduce((a, b) => (b.x > a.x ? b : a))]
      while (picked.length < 8) { let best = sample[0], bd = -1; sample.forEach((s) => { const d = Math.min(...picked.map((q) => q.distanceToSquared(s))); if (d > bd) { bd = d; best = s } }); picked.push(best) }
      const hubs = picked.map((h) => { const q = h.clone().multiplyScalar(0.7); keep(q); return q })
      return { hubs, keep }
    }

    const fromCode = (): Shape => {      // the stand-in: two folded hemispheres, a cerebellum and a brain stem
      const fold = (x: number, y: number, z: number) => Math.sin(x * 5.1 + 1.3) * Math.sin(y * 4.3 + 2.1) * Math.sin(z * 6.2 + 0.7) + 0.55 * Math.sin(x * 11 + z * 9 + 0.4) * Math.sin(y * 8 + 1.1) + 0.3 * Math.sin(z * 17 + y * 13)
      const lobe = (cx: number, cy: number, cz: number, rx: number, ry: number, rz: number, amount: number, detail: number) => {
        let g: THREE.BufferGeometry = new THREE.IcosahedronGeometry(1, detail); g.deleteAttribute('normal'); g.deleteAttribute('uv'); g = mergeVertices(g)
        const p = g.attributes.position as THREE.BufferAttribute; const d = new THREE.Vector3()
        for (let i = 0; i < p.count; i++) { d.set(p.getX(i), p.getY(i), p.getZ(i)).normalize(); const k = 1 + amount * fold(d.x * 1.6, d.y * 1.6, d.z * 1.6); p.setXYZ(i, cx + d.x * rx * k, cy + d.y * ry * k, cz + d.z * rz * k) }
        g.computeVertexNormals(); return g
      }
      const geoms = [lobe(-1.9, 0.2, 0.2, 2.25, 3.0, 4.9, 0.075, 4), lobe(1.9, 0.2, 0.2, 2.25, 3.0, 4.9, 0.075, 4), lobe(0, -2.2, -3.7, 2.3, 1.15, 1.5, 0.11, 3)]
      geoms.forEach((g) => { brain.add(new THREE.Mesh(g, glass)); disposables.push(g) })
      const half = new THREE.Vector3(4.1, 3.2, 4.9)
      const hubs = ([[-0.45, 0.2, 0.55], [0.45, 0.2, 0.55], [-0.55, 0.35, -0.05], [0.55, 0.35, -0.05], [-0.6, -0.25, 0.1], [0.6, -0.25, 0.1], [0, 0.3, -0.62], [0, -0.45, -0.55]] as [number, number, number][]).map((a) => new THREE.Vector3(...a).multiply(half))
      const keep = (q: THREE.Vector3) => { const e = (q.x / half.x) ** 2 + (q.y / half.y) ** 2 + (q.z / half.z) ** 2; if (e > 0.72) q.multiplyScalar(Math.sqrt(0.72 / e)) }
      return { hubs, keep }
    }

    const orbs = new Map<number, THREE.Mesh>(); const labels: { el: HTMLDivElement; pos: THREE.Vector3 }[] = []
    let built = ''; let latest: State | null = null
    const rebuild = (models: Model[]) => {
      if (!shape) return
      const key = models.map((m) => m.n).join(','); if (key === built) return; built = key
      orbs.forEach((o) => brain.remove(o)); orbs.clear(); labels.forEach((l) => l.el.remove()); labels.length = 0
      const groups = [...new Set(models.map((m) => m.group))]
      groups.forEach((g, gi) => {
        const hub = shape!.hubs[gi % shape!.hubs.length]; const colour = new THREE.Color(GROUP_COLOURS[gi % GROUP_COLOURS.length])
        const members = models.filter((m) => m.group === g); const R = 0.55 + 0.1 * Math.sqrt(members.length)
        const label = document.createElement('div'); label.className = 'sd-tag'; label.textContent = g; label.style.color = colour.getStyle(); host.appendChild(label)
        labels.push({ el: label, pos: hub.clone().add(new THREE.Vector3(0, R + 0.45, 0)) })
        members.forEach((m, i) => {
          // the family's dots spread through a small ball (a golden-angle spiral, so they never overlap), kept inside the glass
          const t = (i + 0.5) / members.length; const incl = Math.acos(1 - 2 * t); const az = i * 2.399963
          const r = R * Math.cbrt(0.25 + 0.75 * ((i * 0.618034) % 1))
          const pos = hub.clone().add(new THREE.Vector3(Math.sin(incl) * Math.cos(az), Math.cos(incl) * 0.8, Math.sin(incl) * Math.sin(az)).multiplyScalar(r)); shape!.keep(pos)
          const orb = new THREE.Mesh(new THREE.SphereGeometry(0.13, 16, 12), new THREE.MeshBasicMaterial({ color: colour.clone() })); orb.position.copy(pos)
          orb.userData = { n: m.n, name: m.name, group: g, base: colour.clone(), y: pos.y }; brain.add(orb); orbs.set(m.n, orb)
        })
      })
    }
    const update = (s: State) => {
      latest = s; rebuild(s.models)
      const live = new Set(s.card ? s.card.models : [])   // the models on the current card shimmer
      orbs.forEach((orb, n) => {
        const r = s.ranks[String(n)]; const base = orb.userData.base as THREE.Color; const mat = orb.material as THREE.MeshBasicMaterial
        if (r && r.weight > 0) { const good = r.hearts >= r.skulls; mat.color.copy(base).lerp(new THREE.Color(good ? '#3dff9a' : '#ff3d5a'), Math.min(1, r.weight / 8)); orb.scale.setScalar(1 + Math.min(1.8, r.weight / 5)) }
        else { mat.color.copy(base); orb.scale.setScalar(1) }
        orb.userData.live = live.has(n)
      })
    }
    scene.current = { update }
    let gone = false
    const ready = (s: Shape) => { if (gone) return; shape = s; if (latest) update(latest) }
    new GLTFLoader().load(`${import.meta.env.BASE_URL}models/glass-brain.glb`, (gltf) => {
      let geo: THREE.BufferGeometry | null = null
      gltf.scene.traverse((o) => { const m = o as THREE.Mesh; if (!geo && m.isMesh) geo = m.geometry })
      ready(geo ? fromMesh(geo) : fromCode())
    }, undefined, () => ready(fromCode()))

    const ray = new THREE.Raycaster(); const pointer = new THREE.Vector2(9, 9); let over = false
    const onUi = (t: EventTarget | null) => !!(t as HTMLElement | null)?.closest?.('.sd-left,.sd-right,.sd-modal,.sd-back,button,a,input,select,textarea,.settings-chat,[class*="mc-"]')   // the brain only answers where nothing else is
    let mx = 0, my = 0
    const aim = (e: PointerEvent) => { mx = e.clientX; my = e.clientY; pointer.set((e.clientX / window.innerWidth) * 2 - 1, -(e.clientY / window.innerHeight) * 2 + 1); over = !onUi(e.target) }
    const click = (e: MouseEvent) => { if (onUi(e.target)) return; ray.setFromCamera(pointer, cam); const hit = ray.intersectObjects([...orbs.values()])[0]; if (hit) openNote.current(hit.object.userData.n as number) }
    window.addEventListener('pointermove', aim); window.addEventListener('click', click)
    const size = () => {
      const w = host.clientWidth, h = host.clientHeight; renderer.setSize(w, h, false); cam.aspect = w / h
      cam.setViewOffset(w, h, 0, h * 0.06, w, h)      // the brain stands centred, above the chat bar (the holo table stood below it)
      cam.updateProjectionMatrix()
    }
    const ro = new ResizeObserver(size); ro.observe(host); size()
    let raf = 0; const m0 = motionNow(); const wp = new THREE.Vector3()
    const tick = () => {
      const t = (motionNow() - m0) / 1000
      root.position.y = Math.sin(t * 0.6) * 0.1; root.rotation.y = t * 0.16; root.rotation.z = Math.sin(t * 0.3) * 0.04      // it turns slowly, with the table's gentle float
      orbs.forEach((o, n) => { o.position.y = (o.userData.y as number) + Math.sin(t * 1.2 + n) * 0.05; if (o.userData.live) o.scale.multiplyScalar(1 + Math.sin(t * 5) * 0.012) })
      let hit: THREE.Intersection | undefined
      if (over) { ray.setFromCamera(pointer, cam); hit = ray.intersectObjects([...orbs.values()])[0] }
      setHover(hit ? `#${hit.object.userData.n} ${hit.object.userData.name} (${hit.object.userData.group}): click to read` : '')
      document.body.style.cursor = hit ? 'pointer' : ''
      if (tip.current) { const el = tip.current; if (hit) { el.textContent = `#${hit.object.userData.n} ${hit.object.userData.name}`; el.style.transform = `translate(${Math.min(mx + 14, window.innerWidth - el.offsetWidth - 8)}px,${my + 16}px)`; el.style.opacity = '1' } else el.style.opacity = '0' }
      root.updateMatrixWorld()
      labels.forEach((l) => {
        wp.copy(l.pos).applyMatrix4(brain.matrixWorld); const near = wp.distanceTo(cam.position) < cam.position.length()   // names on the far side of the brain fade
        const p = wp.project(cam); l.el.style.transform = `translate(-50%,-50%) translate(${(p.x * 0.5 + 0.5) * host.clientWidth}px,${(-p.y * 0.5 + 0.5) * host.clientHeight}px)`; l.el.style.opacity = p.z < 1 ? (near ? '1' : '0.35') : '0'
      })
      renderer.render(sc, cam); raf = motionFrame(tick)
    }
    tick()
    return () => { gone = true; cancelMotionFrame(raf); ro.disconnect(); window.removeEventListener('pointermove', aim); window.removeEventListener('click', click); document.body.style.cursor = ''; labels.forEach((l) => l.el.remove()); disposables.forEach((d) => d.dispose()); renderer.dispose(); renderer.domElement.remove(); scene.current = null }
  }, [])
  useEffect(() => { if (state) scene.current?.update(state) }, [state])

  const s = state?.stats; const card = state?.card; const cfg = state?.settings
  const modelButtons = (names: string[], nums: number[], scores?: number[]) => names.map((n, i) => (
    <li key={nums[i]}><button type="button" className="sd-model" onClick={() => void openNote.current(nums[i])} title="Read this mental model"><span>#{nums[i]} {n}</span>{scores ? <em>{Math.round((scores[i] ?? 0) * 100)}%</em> : null}</button></li>
  ))
  const verdicts = (v: Record<string, number>) => Object.entries(v).map(([k, x]) => <span key={k} className={x > 0 ? 'sd-good' : 'sd-bad'} title={`${WHO[k] ?? k}: ${x > 0 ? 'heart' : 'skull'}`}>{WHO[k] ?? k} {x > 0 ? '♥' : '☠'}</span>)
  const lean = dx > 40 ? 'heart' : dx < -40 ? 'skull' : ''

  return (
    <div className="sd">
      <div className="sd-arena" ref={mount} />
      <Link className="sd-back" to="/panels">← Panels</Link>

      <div className="sd-left">
        <div className="sd-eyebrow">Mental models</div>
        <h1 className="sd-title">Swipe Deck</h1>
        <p className="sd-sub">Judge how XIAR picks and uses mental models. A heart means it helped, a skull means it did not.</p>
        <div className="sd-stats">
          <div className="sd-stat" data-hot="true"><span>Combo</span><b>×{s?.combo ?? 0}</b></div>
          <div className="sd-stat"><span>Hearts</span><b className="sd-good">{s?.hearts ?? 0}</b></div>
          <div className="sd-stat"><span>Skulls</span><b className="sd-bad">{s?.skulls ?? 0}</b></div>
          <div className="sd-stat"><span>Level {s?.level ?? 1}</span><b>{(s?.xp ?? 0) % 100}<i>/100 XP</i></b></div>
        </div>
        <div className="sd-quests">
          <div className="sd-quests-top"><span>Daily quests</span><span className="sd-streak" title={`Best run: ${s?.best_streak ?? 0} days`}>Streak {s?.streak ?? 0} day{(s?.streak ?? 0) === 1 ? '' : 's'}</span></div>
          {(s?.quests ?? []).map((q) => (
            <div key={q.id} className="sd-quest" data-done={q.done}>
              <span>{q.done ? '✓ ' : ''}{q.label}</span><em>{q.have}/{q.need}</em>
              <i style={{ width: `${Math.round((q.have / q.need) * 100)}%` }} />
            </div>
          ))}
          <p className="sd-note">+{s?.quest_xp ?? 20} XP for each quest you finish. A vote on any day keeps the streak.</p>
        </div>
        <p className="sd-points">Points: You 3 · Cloud 2 · Local 2 · Agent 1 · Laya 1</p>
        <details className="sd-judges">
          <summary>Judges: who votes</summary>
          <label className="sd-pick"><span>I vote myself</span><input type="checkbox" checked={cfg?.owner ?? true} onChange={(e) => void setJudge('owner', e.target.checked)} /></label>
          <label className="sd-pick"><span>{BOT.cloud}</span>
            <select value={judges?.settings.cloud ?? ''} onChange={(e) => void setJudge('cloud', e.target.value)}>
              <option value="">Off</option>
              {(judges?.connections ?? []).map((c) => <option key={c.id} value={c.id}>{c.id} ({c.where === 'here' ? 'runs here' : 'at a provider'})</option>)}
            </select></label>
          <label className="sd-pick"><span>{BOT.local}</span>
            <select value={judges?.settings.local === 'default' ? '__default' : (judges?.settings.local ?? '')} onChange={(e) => void setJudge('local', e.target.value)}>
              <option value="__default">Default{judges?.local_now ? ` (${judges.local_now}, runs here)` : ''}</option>
              <option value="">Off</option>
              {(judges?.connections ?? []).map((c) => <option key={c.id} value={c.id}>{c.id} ({c.where === 'here' ? 'runs here' : 'at a provider'})</option>)}
            </select></label>
          <p className="sd-note">A bot sees only the task excerpt, the picked model notes and, for an outcome card, the first lines of the agent's answer. Never your vote. A provider connection sends that text to the provider. Each bot that is on goes through every waiting card by itself.</p>
          <button type="button" className="sd-judge" onClick={() => void askBots()} disabled={asking}>{asking ? 'Asking…' : 'Ask the bots now'}</button>
        </details>
        {revealed ? <p className="sd-reveal" role="status">Last card: {revealed}</p> : null}
        {state?.history.length ? <div className="sd-history"><div className="sd-picked">Today's decisions · {s?.finished ?? 0} finished</div>
          <ul>{state.history.map((h) => <li key={h.id}><span className="sd-kind">{KIND[h.kind]}</span><button type="button" className="sd-link" onClick={() => void openNote.current(h.models[0])}>{h.names.join(', ')}</button><span className="sd-verdicts">{verdicts(h.verdicts)}</span></li>)}</ul></div> : <p className="sd-note">Swipe a card to start your log.</p>}
        {error ? <p className="sd-error" role="alert">{error}</p> : null}
      </div>

      <div className="sd-right">
        <HudRings />
        <div className="sd-cardwrap">
          {card ? <div className="sd-card" data-flash={flash ?? ''} data-kind={card.kind} data-lean={lean}
            style={{ transform: dx ? `translateX(${dx}px) rotate(${dx / 18}deg)` : undefined, transition: dx ? 'none' : undefined }}
            onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up}>
            <div className="sd-card-top"><span className="sd-agent">{KIND[card.kind]}</span><span>{card.agent} · {s?.cards_left ?? 0} waiting</span></div>
            <p className="sd-task">{card.task || 'A decision'}</p>
            <div className="sd-picked">{card.kind === 'suggest' ? 'Laya offered' : card.kind === 'use' ? 'The agent chose' : 'The agent decided with'}</div>
            <ul className="sd-models">{modelButtons(card.names, card.models, card.scores)}</ul>
            {card.kind === 'outcome' ? <p className="sd-answer"><b>What it decided:</b> {card.answer || '(no answer kept)'}</p> : null}
            <p className="sd-sealed">{(['cloud', 'local'] as const).filter((k) => cfg?.[k]).map((k) => <span key={k} data-judged={card.sealed.includes(k)}>{BOT[k]}: {card.sealed.includes(k) ? 'judged, sealed' : 'still judging'}</span>)}</p>
          </div> : <div className="sd-card sd-empty">
            <b>{cfg && !cfg.owner ? 'You are not voting' : 'No cards waiting for you'}</b>
            <p>{cfg && !cfg.owner ? 'The bots that are on judge every card for you. The finished cards are in your log.' : (s?.waiting_for_bots ? `The bots are still judging ${s.waiting_for_bots} card${s.waiting_for_bots === 1 ? '' : 's'}; their verdicts go in your log. ` : '') + 'When XIAR or a bot picks, uses or decides with a mental model, the card shows up here.'}</p>
          </div>}
        </div>
        <div className="sd-actions">
          <button type="button" className="sd-skull" onClick={() => void vote('skull')} disabled={!card} aria-label="Skull: bad">☠ Skull</button>
          <button type="button" className="sd-heart" onClick={() => void vote('heart')} disabled={!card} aria-label="Heart: good">♥ Heart</button>
        </div>
        <p className="sd-hint">Swipe right for a heart, left for a skull (or use the arrow keys). The bots judge blind; you see what they said after you vote.</p>
      </div>
      <div className="sd-tip" ref={tip} role="tooltip" />
      <div className="sd-hover" aria-live="polite">{hover || 'Models by family. Green glow: loved. Red glow: skulled. Bigger: more votes. Click one to read it.'}</div>

      {note ? <div className="sd-modal" role="dialog" aria-modal="true" aria-label={note.name} onClick={() => setNote(null)}>
        <div className="sd-modal-box" onClick={(e) => e.stopPropagation()}>
          <div className="sd-modal-top"><b>#{note.n} {note.name}</b><span>{note.group}</span><button type="button" className="sd-judge" onClick={() => setNote(null)} aria-label="Close">Close</button></div>
          <p className="sd-modal-when">{note.when}</p>
          <pre className="sd-modal-note">{note.note}</pre>
        </div>
      </div> : null}
    </div>
  )
}
