"""Summarize one capture or compare two captures; no login or network access."""
import argparse, collections, hashlib, json
from datetime import datetime, timezone, timedelta
from pathlib import Path

def summarize(path):
    path=Path(path)
    if path.is_dir():
        choices=list(path.glob('performance-*.jsonl'))
        if not choices: raise ValueError(f'No performance log in {path}')
        path=max(choices,key=lambda p:p.stat().st_mtime_ns)
    raw=path.read_bytes()
    rows=[json.loads(line) for line in raw.decode('utf-8-sig').splitlines() if line.strip()]
    ds=[d for r in rows for d in r.get('petDetailResponses',{}).get('responses',[])]
    phases=collections.Counter();visits=collections.Counter()
    for d in ds:
        g=d.get('goto',{})
        if g.get('enabled'):
            if abs(sum(p['exclusiveMs'] for p in g['phases'])-g['outerTotalMs'])>1e-6: raise ValueError('Goto stage sum mismatch')
            for p in g['phases']:
                phases[p['name']]+=p['exclusiveMs']
                visits[p['name']]+=p['counts'].get('movieClipScriptVisits',0)
    detail_rows=[r for r in rows if r.get('petDetailResponses',{}).get('responses')]
    summary_rows = [
        r for r in rows
        if r.get('petDetailResponses', {}).get('mode', r.get('detailProfile')) == 'summary'
    ]
    summary_responses = sum(len(r.get('petDetailResponses', {}).get('responses', [])) for r in summary_rows)
    result = dict(source=str(path.resolve()),sha256=hashlib.sha256(raw).hexdigest(),samples=len(rows),
        end=datetime.fromtimestamp(rows[-1]['unixMs']/1000,timezone(timedelta(hours=8))).isoformat(),
        final=rows[-1]['final'],builds=sorted({r['build'] for r in rows}),
        subtree_script_sleep=sorted({r.get('subtreeScriptSleep',False) for r in rows}),
        detail_windows=list({json.dumps(r['window'],sort_keys=True) for r in detail_rows}),
        dropped_responses=sum(r.get('petDetailResponses',{}).get('droppedResponses',0) for r in rows),
        responses=len(ds),detail_ms=sum(d['totalMs'] for d in ds),
        goto_ms=sum(d.get('goto',{}).get('outerTotalMs',0) for d in ds),
        goto_calls=sum(d.get('goto',{}).get('calls',0) for d in ds),phases_ms=phases,script_visits=visits,
        per_response=[dict(sequence=d['sequence'],detail_ms=d['totalMs'],goto_ms=d.get('goto',{}).get('outerTotalMs'),
            calls=d.get('goto',{}).get('calls'),orphan_slots=d.get('goto',{}).get('orphanSlots')) for d in ds])
    if summary_rows:
        # Full response entries keep their own measurements in per_response. A
        # partial set of GOTO timings cannot stand in for the whole capture.
        measured = sum(bool(d.get('goto', {}).get('enabled')) for d in ds)
        result['goto_coverage'] = dict(measured_responses=measured, summary_responses=summary_responses)
        if summary_responses or not ds or measured != len(ds):
            result.update(goto_ms=None, goto_calls=None, phases_ms=None, script_visits=None)
    return result

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--a');parser.add_argument('--b',required=True);parser.add_argument('--output',required=True)
    args=parser.parse_args();result={'B':summarize(args.b)}
    if args.a:result['A']=summarize(args.a)
    result['caution']='Compare only matching scenes/order/windows; totals do not prove causation. Inclusive and exclusive times overlap.'
    out=Path(args.output);out.parent.mkdir(parents=True,exist_ok=True);out.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({k:{n:v[n] for n in ['responses','detail_ms','goto_ms','final']} for k,v in result.items() if k in ['A','B']}))
