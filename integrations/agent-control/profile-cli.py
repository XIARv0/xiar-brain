# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Agent registry operations. Run by the root broker as ROOT (the registry, every private home and every capability are root-owned); only the native control-socket
rescan runs as the owner, because it needs the owner's own ~/.hermes."""
import json,os,pathlib,sys,tomllib
from xiar_hermes.profiles import Profiles,save
root=pathlib.Path(os.environ['XIAR_ROOT']);request=json.load(sys.stdin);p=Profiles(root);command=request['command'];args=request.get('args',{})
if command=='list':result=p.list()
elif command=='upsert':result=p.upsert(args)
elif command=='scope':result=p.provision(args['agent'],args['verified_user'],args['business'])
elif command=='activate':result=p.activate(args['profile'])
elif command=='ready':
    with p.locked():
        state=p.state();cfg=state['agents'][args['agent']];cfg['runtime_ready']=bool(args['ready'])
        p._write_grants(state,exclude_agent=args['agent'])
        for row in state['scopes'].values():
            if row['agent']==args['agent']:
                row['enabled']=cfg['enabled'] and row['business'] in cfg['businesses'] and (not cfg['linux_user'] or cfg['runtime_ready'])
                row['active']=row['enabled'] and (cfg['run_mode']=='always_on' or row['active'])
                p._configure(row,cfg)
        save(p.index,state);p._write_grants(state);result={'ready':cfg['runtime_ready']}
elif command=='freeze':
    with p.locked():
        state=p.state();p._write_grants(state,exclude_agent=args['agent'])
        for row in state['scopes'].values():
            if row['agent']==args['agent']:row['enabled']=False;row['active']=False;p._configure(row,state['agents'][row['agent']])
        save(p.index,state);result={'frozen':True}
elif command=='rescan':
    source=pathlib.Path(tomllib.loads((root/'config/agent_profiles.toml').read_text())['native']['source']).expanduser();sys.path.insert(0,str(source))
    from gateway.control_socket import rescan_gateway_profiles
    result=rescan_gateway_profiles(pathlib.Path.home()/'.hermes')
else:raise ValueError('Unknown private lifecycle operation')
print(json.dumps({'result':result}))
