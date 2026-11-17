"""Compare two fresh, sequential memory trials using the same immutable input."""
import argparse,json,statistics
from pathlib import Path
ap=argparse.ArgumentParser();ap.add_argument('baseline',type=Path);ap.add_argument('candidate',type=Path);ap.add_argument('output',type=Path);args=ap.parse_args()
def read(directory,name):return json.loads((directory/name).read_text())
def metrics(directory):
 r=read(directory,'receipt.json');m=read(directory,'crawler-memory.json');restart=read(directory,'restart-memory.json');assert r['passed'] and not m.get('risk') and not restart.get('risk');assert all(a['samples']>0 for a in m['stages'][:-1]);assert m['settled']
 def peak(stages,key):return max(s['peak'].get(key,0) for s in stages)
 after=[a for a in m['settled'] if a['cycle']>r['changedAt']];assert len(after)>=3
 retained=[a['anon'] for a in after];assert min(retained)>0
 return r,{'rssHighWater':peak(m['stages'],'hwm'),'sampledPeakPrivateAnon':peak(m['stages'],'anon'),'sampledPeakPss':peak(m['stages'],'pss'),'sampledPeakGroupRss':peak(m['stages'],'groupRss'),'sampledPeakHelperRss':peak(m['stages'],'helpers'),'sampledPeakWholeRunnerCgroup':peak(m['stages'],'wholeCgroup'),'restartHighWater':peak([s for s in restart['stages'] if s['name']=='restart-startup'],'hwm'),'completeCycles':r['cycles'],'distanceCheckpoints':r['partials'],'elapsedToLastCompleteMs':m['settled'][-1]['atMs'],'medianIntervalMs':statistics.median(b['atMs']-a['atMs'] for a,b in zip(m['settled'],m['settled'][1:])),'quietAnonAfterChange':retained,'postWarmDrift':retained[-1]-statistics.median(retained[:2])}
a,x=metrics(args.baseline);b,y=metrics(args.candidate);assert a['binarySha256']!=b['binarySha256'];assert a['binarySha256']=='0d9dd6f5be6f8b6f64e4b3145130a83ba9c59034548cf78293d44bf09c2f081a'
for k in ['seedSha256','tier','seedBytes','primaryRoot','fixtureEventIds','signedPublisher','fileHashes','checkpoint','checks','coldNamedRead','oldImmutableRead','upstreamWrites']:assert a[k]==b[k],k
assert min(x['rssHighWater'],y['rssHighWater'],x['restartHighWater'],y['restartHighWater'])>0
# Linux allocator residence is observable only for these trials. These gates use genuine
# completions after warm-up and allow a small host/allocator margin on the smaller tier.
mib=2**20;gates={'unchangedSemanticPayload':True,'postWarmStable':y['postWarmDrift']<=max(48*mib,max(y['quietAnonAfterChange'])*0.10),'restartNoRegression':y['restartHighWater']<=x['restartHighWater']*1.08+16*mib}
if a['tier']=='full':gates['lowerFullGraphPeak']=y['rssHighWater']<=x['rssHighWater']*0.90
else:gates['mobileNoRegression']=y['rssHighWater']<=x['rssHighWater']*1.08+16*mib
report={'tier':a['tier'],'baseline':x,'candidate':y,'seedSha256':a['seedSha256'],'baselineBinary':a['binarySha256'],'candidateBinary':b['binarySha256'],'gates':gates,'passed':all(gates.values()),'peakReductionFraction':1-y['rssHighWater']/x['rssHighWater']};args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'tier':a['tier'],'gates':gates,'peakReductionFraction':report['peakReductionFraction']}));assert report['passed'],gates
