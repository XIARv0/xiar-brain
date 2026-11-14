# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Native profile resolver overlay, installed in memory BEFORE gateway imports.
Upstream code/homes are never written. Keep default/legacy profiles and add only
bound registry homes. Unknown/disabled managed prefixes never fall back to owner.
"""
from __future__ import annotations
import inspect,os,pathlib,sys
from .profiles import Profiles
_INSTALLED=False

def install():
    global _INSTALLED
    if _INSTALLED:return
    from hermes_cli import profiles as native
    from gateway.config import GatewayConfig
    from gateway.run_profile_reconcile import GatewayProfileReconcileMixin
    if 'name' not in inspect.signature(native.get_profile_dir).parameters or 'live_only' not in inspect.signature(native._iter_named_profile_dirs).parameters or not hasattr(GatewayProfileReconcileMixin,'reconcile_served_profiles'):
        raise RuntimeError('Hermes profile contract changed; gateway stays closed until the external adapter is updated')
    runtime=bool(os.environ.get('XIAR_RUNTIME_MANIFEST'))
    if runtime:
        from .runtime_profiles import RuntimeProfiles
        registry=RuntimeProfiles()
    else:registry=Profiles()
    resolve=native.get_profile_dir;iterate=native._iter_named_profile_dirs
    def get_profile_dir(name):
        row=registry.state()['scopes'].get(str(name))
        if row:
            if not runtime and registry.state()['agents'][row['agent']].get('runtime_managed'):raise ValueError('Private profile uses its isolated native listener')
            if not row.get('enabled'):raise ValueError('Private profile disabled')
            return registry.validate(row)
        if str(name).startswith('a-'):raise ValueError('Unknown private profile')
        return resolve(name)
    def iter_dirs(*,live_only=True):
        legacy=iterate(live_only=live_only)
        managed=[registry.validate(row) for row in registry.rows(active=live_only) if runtime or not registry.state()['agents'][row['agent']].get('runtime_managed')]
        if {p.name for p in legacy}&{p.name for p in managed}:raise ValueError('Legacy/private profile identity collision')
        return sorted([*legacy,*managed],key=lambda p:p.name)
    native.get_profile_dir=get_profile_dir;native._iter_named_profile_dirs=iter_dirs
    os.environ['GATEWAY_MULTIPLEX_PROFILES']='false' if runtime else 'true'
    _INSTALLED=True

def external_metadata(root):
    # Entry-point metadata OUTSIDE the Hermes venv survives a native venv rebuild.
    # This is our own package registration, not a pip install into Hermes folders.
    from .profiles import atomic,private_dir
    parent=root/'data/adapter-python';info=parent/'xiar_hermes-0.1.0.dist-info'
    private_dir(info)
    for name,text in {'METADATA':'Metadata-Version: 2.1\nName: xiar-hermes\nVersion: 0.1.0\nSummary: XIAR external native adapter\n',
                      'entry_points.txt':'[hermes_agent.plugins]\nxiar = xiar_hermes.plugin\n'}.items():
        path=info/name
        if not path.is_file() or path.read_text()!=text:atomic(path,text)
    sys.path.insert(0,str(parent))

def main():
    from .config import xiar_root
    import tomllib
    root=xiar_root();runtime=bool(os.environ.get('XIAR_RUNTIME_MANIFEST'))
    if runtime:source=pathlib.Path(os.environ['XIAR_NATIVE_SOURCE'])
    else:
        cfg=tomllib.loads((root/'config/agent_profiles.toml').read_text())
        source=pathlib.Path(cfg['native']['source']).expanduser().resolve()
    if not (source/'hermes_cli/main.py').is_file():raise SystemExit('Native Hermes checkout unavailable')
    sys.path.insert(0,str(source))
    if runtime:sys.path.insert(0,str(root/'integrations/hermes-xiar/distribution'))
    else:external_metadata(root);Profiles(root).reset_on_demand()
    from .model_overlay import install as install_model_overlay
    install_model_overlay(root)
    from .execution import install as install_execution
    install_execution()
    from .autonomy import install as install_autonomy
    install_autonomy()
    install()
    from gateway.run import main as native_main
    from gateway import config as gc,run as gr
    if runtime:
        from .runtime_profiles import RuntimeProfiles
        listener_port=int(RuntimeProfiles().manifest['port'])
    else:
        # Native extra.port takes precedence over API_SERVER_PORT. Apply the
        # owner facade split in memory, never by rewriting Hermes config/home.
        listener=tomllib.loads((root/'config/agent_control.toml').read_text())['control']
        if listener['facade_host']!='127.0.0.1':raise RuntimeError('Native listeners stay loopback until LAN gates pass')
        listener_port=int(listener['owner_port'])
    original=gc.load_gateway_config
    def bounded_config():
        cfg=original();api=cfg.platforms.get(gc.Platform.API_SERVER)
        if api is None:raise RuntimeError('Native listener credentials unavailable')
        api.extra.update(host='127.0.0.1',port=listener_port)
        if runtime:cfg.multiplex_profiles=False
        return cfg
    gc.load_gateway_config=bounded_config
    if hasattr(gr,'load_gateway_config'):gr.load_gateway_config=bounded_config
    sys.argv=['hermes-gateway'];native_main()
if __name__=='__main__':main()
