# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Swipe Deck: the ledger behind the mental-model scoring, and the ranking Laya uses.

Three kinds of card, so the whole memory system learns, not only Laya:
  suggest  Laya's pick: the models it offered the agent for a task (up to three);
  use      the agent's pick: the ONE model the agent actually opened (xiar_mental_model) to make its decision;
  outcome  what came of it: the decision made with that model (the answer's first lines), judged as good or bad.
Every card is judged with a heart or a skull. Judges and their weight:
  owner 3 (the owner, optional)   cloud 2 and local 2 (the two second-opinion bots, each optional; they judge BLIND)
  agent 1 (the agent's own choice of a model = a heart for it)   laya 1 (Laya's own pick = a heart for the models it offered)
  auto 1 (the owner corrected the agent right after: a skull).
Which judges are on is the owner's setting (judges.json): the owner may vote, or leave all of it to the cloud bot, the local bot or both. A card stays in the deck until every
enabled judge has voted (a bot that cannot answer twice is marked skipped, so a card never waits forever); nothing is ever deleted: finished cards move to the history.
Blind judging: a bot is never shown the owner's vote (or another bot's). The owner and the enabled bots get each card at the same time (owner t134u); a bot may judge before or after the owner.
A model's ranking is a smoothed share of weighted hearts (every card counts the same; the weights above are the points). It nudges Laya's score 0.75x to 1.25x, only once 3 weight is
behind the model. Nothing here decides anything: it is a record plus a nudge. Standard library only (the Settings bridge loads this file too).
Files: <state_dir>/swipe-deck/picks.jsonl, votes.jsonl, skips.jsonl, judges.json (mode 600).
"""
from __future__ import annotations

import json
import os
import pathlib
import random
import threading
import time
import uuid

WEIGHT = {"owner": 3.0, "cloud": 2.0, "local": 2.0, "agent": 1.0, "laya": 1.0, "auto": 1.0}     # the owner's points
KINDS = ("suggest", "use", "outcome")
BOTS = ("cloud", "local")
MIN_WEIGHT = 3.0          # a model's ranking counts once this much weight is behind it
EXCERPT = 160             # characters of the task kept with a card
ANSWER = 300              # characters of the agent's answer kept with an outcome card
FAILS_BEFORE_SKIP = 2
QUEST_XP = 20            # bonus XP for each daily quest finished
_LOCK = threading.Lock()


class Deck:
    def __init__(self, state_dir):
        self.dir = pathlib.Path(os.path.expanduser(str(state_dir))) / "swipe-deck"

    # ---- files
    def _append(self, name, record):
        with _LOCK:
            self.dir.mkdir(parents=True, exist_ok=True, mode=0o700)
            fd = os.open(self.dir / name, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
            try:
                os.write(fd, (json.dumps(record, allow_nan=False, ensure_ascii=False) + "\n").encode())
            finally:
                os.close(fd)

    def _read(self, name):
        p = self.dir / name
        if not p.is_file():
            return []
        rows = []
        for line in p.read_text(encoding="utf-8").splitlines():
            try:
                rows.append(json.loads(line))
            except ValueError:
                continue    # a torn last line is skipped, never fatal
        return rows

    def picks(self):
        rows = self._read("picks.jsonl")
        for r in rows:
            r.setdefault("kind", "suggest")
        return rows

    # ---- who judges (the owner's choice)
    def settings(self):
        """owner: bool (default on). cloud / local: a connection id, None (off), or 'default' (local only: the judge module picks the local model that runs here)."""
        try:
            d = json.loads((self.dir / "judges.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            d = {}
        return {"owner": d.get("owner", True) is not False,
                "cloud": (d.get("cloud") or None) if isinstance(d.get("cloud"), (str, type(None))) else None,
                "local": ((d.get("local") or None) if isinstance(d.get("local"), (str, type(None))) else None) if "local" in d else "default"}

    def enabled_bots(self):
        s = self.settings()
        return [r for r in BOTS if s[r]]

    def set_setting(self, key, value):
        if key not in ("owner", "cloud", "local"):
            raise ValueError("unknown judge setting")
        try:
            d = json.loads((self.dir / "judges.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            d = {}
        if key == "local" and value == "__default":
            d.pop("local", None)               # back to the default: the local model that runs here
        else:
            d[key] = bool(value) if key == "owner" else (str(value) if value else "")
        with _LOCK:
            self.dir.mkdir(parents=True, exist_ok=True, mode=0o700)
            tmp = self.dir / "judges.tmp"
            tmp.write_text(json.dumps(d), encoding="utf-8")
            os.chmod(tmp, 0o600)
            os.replace(tmp, self.dir / "judges.json")
        return self.settings()

    # ---- cards
    def _new(self, agent, kind, task, models, scores, parent=None, answer=None):
        pid = uuid.uuid4().hex[:12]
        rec = {"id": pid, "at": time.time(), "agent": str(agent), "kind": kind, "parent": parent, "task": " ".join(str(task).split())[:EXCERPT],
               "models": [int(m) for m in models], "scores": [round(float(s), 4) for s in scores]}
        if answer:
            rec["answer"] = " ".join(str(answer).split())[:ANSWER]
        self._append("picks.jsonl", rec)
        return pid

    def record_pick(self, agent, text, models, scores):
        """Laya's pick: the models it offered the agent for this task."""
        pid = self._new(agent, "suggest", text, models, scores)
        self.vote(pid, 1, "laya")          # Laya's own choice is worth 1 point
        return pid

    def record_use(self, agent, model, within=1800):
        """The agent opened a model to decide with it: its own pick (a heart from the agent, weight 1). Returns the card id, or None if there is nothing to attach it to."""
        now = time.time()
        parents = [p for p in self.picks() if p["agent"] == str(agent) and p["kind"] == "suggest" and now - p["at"] <= within]
        if not parents:
            return None
        parent = parents[-1]
        for p in self.picks():
            if p["kind"] == "use" and p.get("parent") == parent["id"] and p["models"] == [int(model)]:
                return p["id"]
        score = parent["scores"][parent["models"].index(int(model))] if int(model) in parent["models"] else 0.0
        pid = self._new(agent, "use", parent["task"], [int(model)], [score], parent["id"])
        self.vote(pid, 1, "agent")
        return pid

    def record_outcome(self, agent, answer, within=3600):
        """The agent's answer to the task it used a model for: what came of the decision. One per use card."""
        now = time.time()
        picks = self.picks()
        uses = [p for p in picks if p["agent"] == str(agent) and p["kind"] == "use" and now - p["at"] <= within]
        done = {p.get("parent") for p in picks if p["kind"] == "outcome"}
        for u in reversed(uses):
            if u["id"] not in done and str(answer).strip():
                return self._new(agent, "outcome", u["task"], u["models"], u["scores"], u["id"], answer)
        return None

    # ---- votes
    def vote(self, pick_id, value, voter="owner", model=None):
        if voter not in WEIGHT:
            raise ValueError("voter must be owner, cloud, local, agent, laya or auto")
        if value not in (1, -1, "heart", "skull"):
            raise ValueError("a vote is a heart or a skull")
        value = 1 if value in (1, "heart") else -1
        pick = next((p for p in self.picks() if p["id"] == pick_id), None)
        if pick is None:
            raise ValueError("no such pick")
        if model is not None and int(model) not in pick["models"]:
            raise ValueError("that model was not part of this pick")
        votes = self._read("votes.jsonl")
        if voter in BOTS:          # blind judging: a bot never sees anyone's vote, and judges each card only once
            if any(v["pick"] == pick_id and v["voter"] == voter for v in votes):
                raise ValueError("that bot has already judged this pick")
        if voter == "owner" and any(v["pick"] == pick_id and v["voter"] == "owner" for v in votes):
            raise ValueError("the owner has already voted on this pick")
        self._append("votes.jsonl", {"pick": pick_id, "at": time.time(), "voter": voter, "value": value, "model": None if model is None else int(model)})
        return {"pick": pick_id, "voter": voter, "value": value}

    def bots_on(self, pick_id):
        """What each bot said about this pick so far (the panel keeps it sealed until the owner has voted)."""
        return {v["voter"]: v["value"] for v in self._read("votes.jsonl") if v["pick"] == pick_id and v["voter"] in BOTS}

    def note_failure(self, pick_id, role):
        """A bot could not answer. After two failures it is skipped for this card, so the card never waits forever."""
        self._append("skips.jsonl", {"pick": pick_id, "role": role, "at": time.time()})
        return sum(1 for s in self._read("skips.jsonl") if s["pick"] == pick_id and s["role"] == role)

    def _skipped(self, pick_id, role, skips=None):
        return sum(1 for s in (skips if skips is not None else self._read("skips.jsonl")) if s["pick"] == pick_id and s["role"] == role) >= FAILS_BEFORE_SKIP

    def auto_skull_latest(self, agent, within=1800):
        """The owner corrected the agent right after its decision: the latest card of that decision gets an automatic skull (once)."""
        picks = [p for p in self.picks() if p["agent"] == str(agent) and time.time() - p["at"] <= within]
        if not picks:
            return None
        last = picks[-1]
        if any(v["pick"] == last["id"] and v["voter"] == "auto" for v in self._read("votes.jsonl")):
            return None
        return self.vote(last["id"], -1, "auto")

    # ---- where each card stands
    def _status(self, pick, votes, skips, settings):
        voted = {v["voter"] for v in votes if v["pick"] == pick["id"]}
        bots = [r for r in BOTS if settings[r]]
        waiting_bots = [r for r in bots if r not in voted and not self._skipped(pick["id"], r, skips)]
        owner_needed = settings["owner"] and "owner" not in voted
        return {"waiting_bots": waiting_bots, "owner_needed": owner_needed, "ready_for_owner": owner_needed,
                "complete": not waiting_bots and not owner_needed}

    def pending_for(self, role):
        """Cards this bot still has to judge: it is on and has not voted or been skipped (whether or not the owner has voted)."""
        settings, votes, skips = self.settings(), self._read("votes.jsonl"), self._read("skips.jsonl")
        if not settings[role]:
            return []
        return [p for p in self.picks() if role in self._status(p, votes, skips, settings)["waiting_bots"]]

    def next_card(self, voter="owner"):
        """The oldest card this voter can judge now. The owner sees a card as soon as it is made, at the same time as the bots."""
        settings, votes, skips = self.settings(), self._read("votes.jsonl"), self._read("skips.jsonl")
        if voter == "owner":
            for p in self.picks():
                if self._status(p, votes, skips, settings)["ready_for_owner"]:
                    return p
            return None
        done = {v["pick"] for v in votes if v["voter"] == voter}
        return next((p for p in self.picks() if p["id"] not in done), None)

    def history(self, limit=12):
        """Finished cards, newest first, with who said what."""
        settings, votes, skips = self.settings(), self._read("votes.jsonl"), self._read("skips.jsonl")
        out = []
        for p in reversed(self.picks()):
            if self._status(p, votes, skips, settings)["complete"] and any(v["pick"] == p["id"] for v in votes):
                out.append(dict(p, verdicts={v["voter"]: v["value"] for v in votes if v["pick"] == p["id"]}))
                if len(out) >= limit:
                    break
        return out

    # ---- ranking
    def ranks(self):
        picks = {p["id"]: p for p in self.picks()}
        tally = {}
        for v in self._read("votes.jsonl"):
            p = picks.get(v["pick"])
            if not p:
                continue
            w = WEIGHT.get(v["voter"], 1.0)
            for m in ([v["model"]] if v.get("model") is not None else p["models"]):
                t = tally.setdefault(int(m), {"up": 0.0, "down": 0.0})
                t["up" if v["value"] > 0 else "down"] += w
        out = {}
        for m, t in tally.items():
            w = t["up"] + t["down"]
            share = (t["up"] + 1) / (w + 2)                      # smoothed share of hearts
            out[m] = {"hearts": round(t["up"], 2), "skulls": round(t["down"], 2), "weight": round(w, 2), "factor": round(0.75 + 0.5 * share, 4) if w >= MIN_WEIGHT else 1.0}
        return out

    def factor(self, model, ranks=None):
        return (ranks if ranks is not None else self.ranks()).get(int(model), {}).get("factor", 1.0)

    # ---- the owner's score, daily quests and streak (days are the machine's local days)
    @staticmethod
    def _daily(owner, by_pick):
        daily = {}
        for v in owner:
            d = daily.setdefault(time.strftime("%Y-%m-%d", time.localtime(v["at"])), {"votes": 0, "agree": 0})
            d["votes"] += 1
            bots = {k: x for k, x in by_pick.get(v["pick"], {}).items() if k in BOTS}
            if any(x == v["value"] for x in bots.values()):
                d["agree"] += 1
        return daily

    @staticmethod
    def _streak(daily, today):
        """Days in a row with at least one vote by the owner, ending today (or yesterday while today has none yet), and the best run ever."""
        days = sorted(daily)
        best = run = 0
        prev = None
        for k in days:
            cur = time.mktime(time.strptime(k + " 12", "%Y-%m-%d %H"))   # noon: a daylight-saving change never skips a day
            run = run + 1 if prev is not None and round((cur - prev) / 86400) == 1 else 1
            best = max(best, run)
            prev = cur
        t0 = time.mktime(time.strptime(today + " 12", "%Y-%m-%d %H"))
        streak, k = 0, t0 if today in daily else t0 - 86400
        while time.strftime("%Y-%m-%d", time.localtime(k)) in daily:
            streak += 1
            k -= 86400
        return streak, best

    def stats(self):
        votes = self._read("votes.jsonl")
        owner = [v for v in votes if v["voter"] == "owner"]
        run = 0
        for v in reversed(owner):                                 # combo: hearts in a row since the last skull
            if v["value"] < 0:
                break
            run += 1
        by_pick = {}
        for v in votes:
            by_pick.setdefault(v["pick"], {}).setdefault(v["voter"], v["value"])
        agree = sum(1 for d in by_pick.values() if "owner" in d and any(k in BOTS and x == d["owner"] for k, x in d.items()))
        settings, skips = self.settings(), self._read("skips.jsonl")
        st = [self._status(p, votes, skips, settings) for p in self.picks()]
        cards_left = sum(1 for s in st if s["ready_for_owner"])
        waiting = sum(1 for s in st if s["waiting_bots"])                          # cards the bots are still judging
        daily = self._daily(owner, by_pick)
        today = time.strftime("%Y-%m-%d")
        day = daily.get(today, {"votes": 0, "agree": 0})
        quests = [{"id": "judge5", "label": "Judge 5 cards", "have": min(day["votes"], 5), "need": 5},
                  {"id": "agree3", "label": "Agree with a bot 3 times", "have": min(day["agree"], 3), "need": 3},
                  {"id": "clear", "label": "Clear the deck", "have": 1 if (day["votes"] >= 1 and cards_left == 0 and waiting == 0) else 0, "need": 1}]
        for q in quests:
            q["done"] = q["have"] >= q["need"]
        bonus = QUEST_XP * (sum((d["votes"] >= 5) + (d["agree"] >= 3) for k, d in daily.items() if k != today) + sum(q["done"] for q in quests))
        streak, best = self._streak(daily, today)
        xp = 10 * len(owner) + 5 * agree + bonus
        return {"quests": quests, "streak": streak, "best_streak": best, "bonus_xp": bonus, "quest_xp": QUEST_XP,"hearts": sum(1 for v in owner if v["value"] > 0), "skulls": sum(1 for v in owner if v["value"] < 0), "combo": run,
                "agreements": agree, "xp": xp, "level": 1 + xp // 100, "cards_left": cards_left,
                "waiting_for_bots": waiting,
                "finished": sum(1 for s in st if s["complete"]), "bot_votes": sum(1 for v in votes if v["voter"] in BOTS)}


class ForecastDeck(Deck):
    """Forecast cards (UpgradePlan 8.13, owner t132u): the same ledger, judges and points, kept in its own
    folder (<forecast state>/swipe-deck) so the mental-model ranking is never mixed with the forecasting one.
      suggest  Laya's pick: the forecasting models she ranked highest for a series;
      use      the forecast: the model(s) used and the forecast itself;
      outcome  what really happened, with an automatic heart or skull from the truth score.
    The judges (owner, cloud bot, local bot) are the Swipe Deck's own setting. Model numbers on a card are
    positions in model-index.json (append-only), so a removed model keeps its history."""

    def __init__(self, xiar_state_dir, forecast_state_dir):
        self.main = Deck(xiar_state_dir)
        self.dir = pathlib.Path(os.path.expanduser(str(forecast_state_dir))) / "swipe-deck"

    def settings(self):
        return self.main.settings()

    def set_setting(self, key, value):
        return self.main.set_setting(key, value)

    def index(self):
        try:
            ids = json.loads((self.dir / "model-index.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            ids = []
        return {m: i for i, m in enumerate(ids)}

    def idx(self, model_id):
        with _LOCK:
            idx = self.index()
            if model_id not in idx:
                ids = sorted(idx, key=idx.get) + [str(model_id)]
                self.dir.mkdir(parents=True, exist_ok=True, mode=0o700)
                tmp = self.dir / "model-index.tmp"
                tmp.write_text(json.dumps(ids), encoding="utf-8")
                os.replace(tmp, self.dir / "model-index.json")
                return len(ids) - 1
            return idx[model_id]

    def names(self, models):
        """{number: name} for card display; models: the pool's entries."""
        by_id = {m["id"]: m.get("name", m["id"]) for m in models}
        return {i: by_id.get(m, m) for m, i in self.index().items()}

    def _card(self, agent, kind, task, model_ids, scores, forecast, parent=None, answer=None):
        pid = uuid.uuid4().hex[:12]
        rec = {"id": pid, "at": time.time(), "agent": str(agent), "kind": kind, "parent": parent, "forecast": forecast,
               "task": " ".join(str(task).split())[:EXCERPT], "models": [self.idx(m) for m in model_ids], "scores": [round(float(s), 4) for s in scores]}
        if answer:
            rec["answer"] = " ".join(str(answer).split())[:ANSWER]
        self._append("picks.jsonl", rec)
        return pid

    def forecast_cards(self, record, out):
        """Laya's pick (her top three, a Laya heart) and the forecast (an agent heart: the caller relied on it)."""
        what = record.get("question") or ("a %d-point series, %d steps ahead" % (record["features"]["length"], record["horizon"]))
        probs = record.get("probs") or {}
        top = sorted(probs, key=lambda m: -probs[m])[:3] or out["models"]
        pick = self._card(record["caller"], "suggest", what, top, [probs.get(m, 1.0) for m in top], record["id"])
        self.vote(pick, 1, "laya")
        med = out["median"]
        lo, hi = out["quantiles"][min(out["quantiles"], key=float)], out["quantiles"][max(out["quantiles"], key=float)]
        answer = "Next %d steps: %s to %s (median); first step %.4g (range %.4g to %.4g). %s" % (
            len(med), "%.4g" % med[0], "%.4g" % med[-1], med[0], lo[0], hi[0], out.get("rationale", ""))
        use = self._card(record["caller"], "use", what, out["models"], [probs.get(m, 1.0) for m in out["models"]], record["id"], pick, answer)
        self.vote(use, 1, "agent")
        return pick, use

    def outcome(self, record, score):
        use = next((p for p in self.picks() if p.get("forecast") == record["id"] and p["kind"] == "use"), None)
        if use is None or any(p.get("forecast") == record["id"] and p["kind"] == "outcome" for p in self.picks()):
            return None
        f = score["final"]
        answer = "Real values: %s. Error %.2f times a naive guess (MASE), %d%% inside the forecast range." % (
            ", ".join("%.4g" % v for v in score["actual"][:6]) + ("..." if len(score["actual"]) > 6 else ""), f["mase"], round(100 * f["coverage"]))
        out = self._card(record["caller"], "outcome", use["task"], record["used"], use["scores"], record["id"], use["id"], answer)
        self.vote(out, 1 if score["heart"] else -1, "auto")
        return out

    def verdicts(self):
        """{forecast id: +1 / -1}: the owner's and bots' weighted net verdict on Laya's pick card."""
        picks = {p["id"]: p for p in self.picks() if p["kind"] == "suggest"}
        net = {}
        for v in self._read("votes.jsonl"):
            p = picks.get(v["pick"])
            if p and v["voter"] in ("owner", "cloud", "local"):
                net[p["forecast"]] = net.get(p["forecast"], 0.0) + WEIGHT[v["voter"]] * v["value"]
        return {k: (1 if x > 0 else -1) for k, x in net.items() if x}

    # what the judging bots are sent for a forecast card (swipe_judge uses these when a deck has them)
    def judge_models(self, root, pick):
        names = {}
        try:
            import tomllib
            cfg = tomllib.loads((pathlib.Path(root) / "config/forecasting.toml").read_text(encoding="utf-8"))
            names = {m["id"]: m for m in cfg.get("model", [])}
        except (OSError, ValueError):
            pass
        rev = {i: m for m, i in self.index().items()}
        return [{"name": names.get(rev.get(n), {}).get("name", rev.get(n, "#%d" % n)), "when": "", "note": names.get(rev.get(n), {}).get("about", "")} for n in pick["models"]]

    @staticmethod
    def judge_messages(pick, models):
        kind = pick.get("kind", "suggest")
        rows = "\n".join("%s: %s" % (m["name"], m.get("note", "")) for m in models)
        system = "You judge forecasts of numbers over time and the forecasting models used for them. Answer with exactly one word: HEART if it was good, SKULL if it was bad."
        ask = {"suggest": "A system picked these forecasting models for the series. Were they sensible models to pick?",
               "use": "This forecast was made with these models. Does it look like a sensible forecast for the series?",
               "outcome": "This forecast has now been checked against what really happened. Was it a good forecast?"}.get(kind)
        user = "%s\n\nSeries: %s\n\nModels:\n%s\n" % (ask, pick.get("task") or "(no description)", rows)
        if pick.get("answer"):
            user += "\n%s\n" % pick["answer"]
        return [{"role": "system", "content": system}, {"role": "user", "content": user + "\nHEART or SKULL?"}]