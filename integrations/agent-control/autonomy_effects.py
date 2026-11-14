# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""What each logged owner action can honestly be undone with. Pure functions: no I/O, so they can be tested anywhere.

A logged-and-undoable action (tier `logged_undoable`) is undone in one of three exact ways, and nothing else is ever advertised as reversible:
  files     the action only rewrites bounded, non-secret configuration files: the coordinator keeps sealed before/after images and restores them
            (`[undo.native_paths]` in the root policy; autonomy_undo.py).
  inverse   an exact, idempotent opposite action exists (start <-> stop, add <-> remove, map <-> unmap ...). Where the opposite needs the value that was
            replaced, a read-only `capture` runs BEFORE the action and the plan is built from that before-state.
  none      everything else: the reason is stated, so the owner and Xiar never believe an effect can be rolled back when it cannot (downloads removed,
            derived data, data deleted from a store, services started, a deleted Agent).
The snapshot taken before every logged action is a recovery aid for the owner, not an undo promise."""

# ---- actions whose opposite is exact. Each entry: (inverse tool, function(args) -> args of the inverse, or None when the args are not enough).
def _id(args):return {'id':args['id']} if isinstance(args.get('id'),str) and args['id'] else None
def _path(args):return {'path':args['path']} if isinstance(args.get('path'),str) and args['path'] else None
def _found_id(args):
    found=args.get('found')
    return {'id':found['id']} if isinstance(found,dict) and isinstance(found.get('id'),str) else None

INVERSE={
    'connection_start':('connection_stop',_id),
    'connection_stop':('connection_start',_id),
    'modelpaths_add':('modelpaths_remove',_path),
    'modelpaths_remove':('modelpaths_add',_path),
    'modelfind_map':('modelfind_unmap',_found_id),
    'voice_install':('voice_remove',_id),
    'voice_remove':('voice_install',_id),     # downloads again from the original source and checks the hash; needs the network
}

# ---- actions whose opposite needs the value they replace: a read-only call made before the action, and a planner that turns its result into the inverse.
CAPTURE={
    'voice_select':('voice_catalogue',{}),
    'voice_noncommercial':('voice_catalogue',{}),
    'voice_chatterbox_set':('voice_catalogue',{}),
    'voice_agent_set':('voice_status',{}),
    'modelfind_unmap':('modelfind_mapped',{}),
}

# ---- actions that cannot be undone, with the reason shown to the owner
NOT_REVERSIBLE={
    'memory_reindex':'It rebuilds derived search data. Nothing is lost, but there is no earlier index to put back; run it again to rebuild.',
    'ontology_build':'It rebuilds the ontology from the stored facts. There is no earlier ontology image to restore; build again to regenerate it.',
    'workspace_delete':'It deletes a model or Agent definition (and for an Agent its private registry entry). The definition is not captured, so it cannot be put back.',
    'chatbot_delete':'It deletes a bot definition from the Open WebUI fork. The definition is not captured, so it cannot be put back.',
    'agent_activate':'It starts an Agent service. Stop the Agent from Settings > Maintenance; there is no automatic undo of a started process.',
    'memory_view_delete':'It deletes a stored memory entry. The entry is not kept.',
    'knowledge_remove':'It removes a document and its facts from memory. They are not kept.',
    'knowledge_add':'It adds a document to memory; remove it from the knowledge list.',
}

def _selected(entries,kind):
    for entry in entries or []:
        if isinstance(entry,dict) and entry.get('kind')==kind and entry.get('selected'):return entry
    return None

def plan_inverse(tool,args,before=None):
    """The exact opposite steps for one executed action as [{'tool','args'}], or None when there is none. `before` is the read-only capture result."""
    if not isinstance(args,dict):return None
    if tool in INVERSE:
        name,build=INVERSE[tool];inverse=build(args)
        return [dict(tool=name,args=inverse)] if inverse is not None else None
    if tool not in CAPTURE or not isinstance(before,dict):return None
    if tool=='voice_select':
        target=next((e for e in before.get('entries',[]) if isinstance(e,dict) and e.get('id')==args.get('id')),None)
        previous=_selected(before.get('entries'),target.get('kind') if target else None)
        if not previous:return None
        return [dict(tool='voice_select',args={'id':previous['id']})]
    if tool=='voice_noncommercial':
        return [dict(tool='voice_noncommercial',args={'accepted':bool(before.get('show_noncommercial'))})]
    if tool=='voice_chatterbox_set':
        chatterbox=before.get('chatterbox') if isinstance(before.get('chatterbox'),dict) else {}
        return [dict(tool='voice_chatterbox_set',args={'url':str(chatterbox.get('url',''))})]
    if tool=='voice_agent_set':
        agent=args.get('agent');state=before.get('state') if isinstance(before.get('state'),dict) else {}
        agents=state.get('agents') if isinstance(state.get('agents'),dict) else {}
        row=agents.get(agent) if isinstance(agent,str) else None
        return [dict(tool='voice_agent_set',args={'agent':agent,'id':row['id'] if isinstance(row,dict) and isinstance(row.get('id'),str) else ''})]
    if tool=='modelfind_unmap':
        mapped=before.get('mapped') if isinstance(before.get('mapped'),list) else []
        row=next((m for m in mapped if isinstance(m,dict) and isinstance(m.get('found'),dict) and m['found'].get('id')==args.get('id')),None)
        return [dict(tool='modelfind_map',args={'found':row['found']})] if row else None
    return None

def describe(tool,native_paths):
    """How this action can be undone: {'reversible': 'files' | 'inverse' | 'none', 'how': text}. `native_paths` is the root policy's [undo.native_paths]."""
    if native_paths.get(tool):
        return dict(reversible='files',how='The configuration file%s it changes (%s) are restored from the saved before image. Services pick the restored settings up when they next restart.'%('s' if len(native_paths[tool])>1 else '',', '.join(native_paths[tool])))
    if tool in INVERSE:
        return dict(reversible='inverse',how='Undone by running the exact opposite action (%s).'%INVERSE[tool][0])
    if tool in CAPTURE:
        return dict(reversible='inverse',how='The previous value is read before the change and put back by the same action (%s).'%tool)
    return dict(reversible='none',how=NOT_REVERSIBLE.get(tool,'No exact way to undo this action is defined, so none is offered.'))

def classify_orphan(before,current,same):
    """A transaction that began and never finished (the process died between). `before` / `current` are captures of the files it could touch.
    Returns ('failed', note) when nothing changed, ('uncertain', note) when something did: the effect cannot be called a success or a failure."""
    if len(before)==len(current) and all(same(c,b) for c,b in zip(current,before)):return 'failed','The action never changed its files.'
    return 'uncertain','The action began and never recorded its end, and its files differ from the saved before image. Review it and undo it if it is unwanted.'
