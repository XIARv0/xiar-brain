# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
"""Jobs 6, 5 and 4 regression cases. Run in the final integrated test pass."""
import copy
import json
import pathlib
import sys
import pytest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from xiar_hermes.config import load, AdapterConfig
from xiar_hermes.decisions import probability, score
from xiar_hermes.mental_models import MentalModels
from xiar_hermes.thinking import ThinkingModels
from xiar_hermes.corrections import Corrections
from xiar_hermes.retrieval import Retrieval
from xiar_hermes.plugin import XiarAdapter

ROOT=pathlib.Path(__file__).resolve().parents[3]

@pytest.fixture
def cfg(tmp_path):
    c=load(str(ROOT/'config/hermes_adapter.toml'));raw=copy.deepcopy(c.raw)
    raw['routing']['state_dir']=str(tmp_path)
    raw['laya']['jobs']['memory_routing']=False
    raw['mental_models']['dir']=str(ROOT/'Brain/Shared/mental-models')
    raw['mental_models']['judge']=False   # no background judging threads in unit tests
    return AdapterConfig(raw,ROOT)

class Brain:
    def __init__(self): self.lessons=[];self.queries=[]
    def lesson(self,l): self.lessons.append(l);return True
    def signals(self,c,a): self.queries.append((c,a));return [dict(label='prefer',text='owner prefers',why='plain paragraphs',confidence=.9)]

class Memory:
    def __init__(self): self.writes=[];self.reads=[]
    def insert(self,b,*a,**kw): self.writes.append((b,a,kw));return dict(id='one')
    def search(self,b,q,k,**kw):
        self.reads.append((b,q,k,kw));return dict(business_id=b,agent_id='xiar',end_user_id='owner',hits=[dict(id='one',name='vendor',summary='sensors')])

class Laya:
    def __init__(self,kind=0,p=.95,strategy=1,down=False): self.kind=kind;self.p=p;self.strategy=strategy;self.down=down;self.asked=[]
    def decide(self,text,qs):
        self.asked.append(qs)
        if self.down: return None
        out=[]
        for q in qs:
            if q['type']=='score': out.append(dict(level_probabilities=[0,0,0,.2,.8],value=3.8))
            elif q['type']=='noul':out.append(dict(probability=self.p))
            else:
                count=len(q['options']);n=self.strategy if count==5 else self.kind if 'latest user message' in q['instructions'] else 0
                out.append(dict(probabilities=[.97 if i==n else .03/(count-1) for i in range(count)]))
        return out

@pytest.mark.parametrize('bad',[float('nan'),float('inf'),True,-.1,1.1,'0.9'])
def test_bad_probabilities_refused(bad):
    with pytest.raises(ValueError): probability(bad)

def test_laya_decides_from_all_models_in_two_steps_with_bounded_batches(cfg):
    l=Laya();catalogue=MentalModels(cfg.path('mental_models.dir'));job=ThinkingModels(cfg,l,catalogue)
    selected=job.select('Evaluate the deployment risks')
    assert len(catalogue)==99 and len(selected)==3
    assert len(l.asked[0])==8 and all('family of mental models' in q['instructions'] for q in l.asked[0])      # step 1: the 8 families, one batch
    models_scored=sum(map(len,l.asked[1:]))
    sizes=sorted((sum(1 for m in catalogue.models if m.group==g) for g in {m.group for m in catalogue.models}),reverse=True)
    assert max(map(len,l.asked))<=8 and models_scored<=sum(sizes[:2])+2 and models_scored>=job.explore    # step 2: the best 2 families + 2 from the others, never all 99
    assert len(job.last_families)==2 and [m.num for m in selected]==[1,2,3]

def test_no_lexical_fallback_on_laya_failure(cfg):
    assert ThinkingModels(cfg,Laya(down=True),MentalModels(cfg.path('mental_models.dir'))).select('risk of failure')==[]

@pytest.mark.parametrize('kind,expected',[(0,'brain'),(1,'semantic'),(2,'semantic'),(3,'none')])
def test_corrections_routes_without_approval_queue(cfg,kind,expected):
    b=Brain();m=Memory();r=Corrections(cfg,Laya(kind=kind),b,m).handle('No, use the requested format.','Wrong format.','tenant-a')
    assert bool(b.lessons)==(expected=='brain')
    assert bool(m.writes)==(expected=='semantic')
    if b.lessons:
        assert b.lessons[0]['kind']=='correction' and 'tenant-a' not in json.dumps(b.lessons)

def test_correction_missing_business_does_not_enter_brain(cfg):
    b=Brain();m=Memory();r=Corrections(cfg,Laya(kind=1),b,m).handle('The warehouse opens at seven.','It opens at nine.')
    assert not b.lessons and not m.writes and r[0]['reason']=='business_required_no_write'

def test_only_xiar_can_learn(cfg):
    raw=copy.deepcopy(cfg.raw);raw['identity']['agent_id']='scout';c=AdapterConfig(raw,cfg.root)
    b=Brain();Corrections(c,Laya(),b,Memory()).handle('No, use paragraphs.','Bullets.')
    assert not b.lessons
    a=XiarAdapter(c,brain=b,memory=Memory(),laya=Laya(),background=False)
    assert not a._lesson(kind='correction',outcome='corrected',action='reply',why='paragraphs')

def test_past_business_reply_is_not_copied_to_lesson(cfg):
    b=Brain();Corrections(cfg,Laya(),b,Memory()).handle('Stop inventing requirements.','Business confidential previous response')
    assert 'confidential' not in json.dumps(b.lessons)

def test_below_threshold_and_down_never_write(cfg):
    for l in (Laya(p=.1),Laya(down=True)):
        b=Brain();m=Memory();Corrections(cfg,l,b,m).handle('No, use paragraphs.','Bullets.','tenant-a')
        assert not b.lessons and not m.writes

def test_secret_looking_correction_not_sent_to_laya_or_writer(cfg):
    l=Laya();b=Brain();Corrections(cfg,l,b,Memory()).handle('No, password=TEST-FIXTURE-NOT-A-CREDENTIAL')
    assert not l.asked and not b.lessons

@pytest.mark.parametrize('strategy,ontologies,signals',[(0,[False],False),(1,[True],False),(2,[],True),(3,[False,True],False),(4,[True],True)])
def test_retrieval_strategies_keep_one_scope(cfg,strategy,ontologies,signals):
    b=Brain();m=Memory();r=Retrieval(cfg,Laya(strategy=strategy),m,b).search('tenant-a','Find remembered information',8)
    assert [x[3]['ontology'] for x in m.reads]==ontologies
    assert all(x[0]=='tenant-a' for x in m.reads)
    assert bool(r['signals'])==signals
    assert len(r['hits'])<=1  # mixed reads deduplicate provenance ids

def test_retrieval_no_business_no_laya(cfg):
    l=Laya();r=Retrieval(cfg,l,Memory(),Brain()).search('','vendors',8)
    assert 'error' in r and not l.asked

def test_assistant_response_contract_retained(cfg):
    a=XiarAdapter(cfg,brain=Brain(),memory=Memory(),laya=Laya(),background=False)
    a.post_llm_call(session_id='s',assistant_response='previous answer')
    assert a.last_reply['s']=='previous answer'
    a.on_session_end('s');assert 's' not in a.last_reply