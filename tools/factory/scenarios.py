#!/usr/bin/env python3
"""Search reproducible scenario recipes without touching the running kiosk.

python3 tools/factory/scenarios.py generate --out results/scenario-suite
python3 tools/factory/scenarios.py run --out results/scenario-suite
python3 tools/factory/scenarios.py report --out results/scenario-suite
"""
import argparse
import hashlib
import itertools
import json
import os
from pathlib import Path
import statistics
import struct
import subprocess

ROOT = Path(__file__).resolve().parents[2]
POLICIES = ('none', 'ignition', 'evacuate', 'adaptive')
# Distinct geography, with offset starts on either side of the ridge.
STARTS = [
    ('ovest', 'Le Ghiande', (3438, 3149), 180),
    ('valico', 'Le Terrazze', (5590, 4370), 119),
    ('est', 'Castelvento', (5870, 4750), 119),
]
# Piecewise-constant fuel moisture, not atmospheric relative humidity.
PROFILES = {
    'regolare': [(0, 0, 24, 9), (90, 0, 20, 11)],
    'rinforzo_secco': [(0, 0, 24, 9), (60, -25, 36, 5), (150, -40, 40, 4)],
    'rotazione': [(0, 0, 32, 6), (45, -69, 36, 5), (120, 20, 28, 7)],
    'rientro_umido': [(0, 0, 38, 5), (60, -40, 32, 6), (120, -70, 18, 12)],
}


def dump(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(obj, ensure_ascii=False, indent=2) + '\n')
    temporary.replace(path)


def fingerprint():
    """Hash the headless model and its inputs, independent of kiosk rendering.

    score.rs formats/ranks completed outcomes; the probe never invokes it.
    Tests, other examples and graphical crates are not simulation inputs.
    """
    crates = ('rocca','fire','abm','scenario','behavior','datafs')
    paths = []
    for crate in crates:
        directory = ROOT/'crates'/crate
        paths.append(directory/'Cargo.toml')
        paths.extend(p for p in (directory/'src').rglob('*.rs')
                     if 'bin' not in p.relative_to(directory/'src').parts
                     and p != ROOT/'crates/rocca/src/score.rs')
        if (directory/'build.rs').exists():
            paths.append(directory/'build.rs')
    paths.append(ROOT/'crates/rocca/examples/scenario_suite.rs')
    for base in ('data/scenarios/rocca_ventosa','data/behaviours'):
        paths.extend(p for p in (ROOT/base).rglob('*') if p.is_file())
    paths += [ROOT/'data/fuels_eu12.json',ROOT/'Cargo.lock',ROOT/'Cargo.toml',ROOT/'.cargo/config.toml']
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(ROOT)).encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()


def generate(out, playable=False, impacts=False):
    if impacts:
        return generate_impacts(out)
    if playable:
        return generate_playable(out)
    data = ROOT / 'data/scenarios/rocca_ventosa'
    fuel = struct.unpack('<160000i', (data / 'fuel.i32').read_bytes())
    recipes = []
    for (zone, near, (x, y), direction), (profile, history) in itertools.product(STARTS, PROFILES.items()):
        # Reject unsuitable starts instead of silently clamping/snapping to sea.
        assert 0 <= x < 8000 and 0 < y < 8000
        assert fuel[int((8000-y)//20)*400 + int(x//20)] > 0
        points = [dict(at_min=t, wind_from_deg=(direction+d) % 360,
                       wind_kmh=speed, moisture_pct=moisture) for t, d, speed, moisture in history]
        name = f'{zone}_{profile}'
        recipes.append(dict(zone=zone, profile=profile, case=dict(
            name=name, near=near, ignition=[x, y], radius_m=60,
            wind_from_deg=points[0]['wind_from_deg'], wind_kmh=points[0]['wind_kmh'],
            moisture_pct=points[0]['moisture_pct'], minutes=240, start_min=30), history=points))
    dump(out / 'recipes.json', dict(schema=1, recipes=recipes))
    print(f'{len(recipes)} ricette in {out / "recipes.json"}', flush=True)


def generate_playable(out):
    """Candidates that run unmodified through the kiosk's Case/weather model."""
    starts = [
        ('ovest', 'Le Ghiande', (3600, 2700), 150),
        ('valico', 'Le Terrazze', (5590, 4370), 119),
        ('est', 'Castelvento', (5870, 4750), 119),
    ]
    profiles = {
        'brezza_umida': (24, 9, None),
        'brezza_tesa': (28, 7, None),
        'rinforzo': (26, 7, (75, -25, 34)),
        'giro_moderato': (28, 6, (75, -69, 32)),
    }
    fuel = struct.unpack('<160000i', (ROOT/'data/scenarios/rocca_ventosa/fuel.i32').read_bytes())
    recipes = []
    for (zone,near,(x,y),direction),(profile,(speed,moisture,shift)) in itertools.product(starts,profiles.items()):
        assert fuel[int((8000-y)//20)*400+int(x//20)] > 0
        case = dict(name=f'giocabile_{zone}_{profile}',near=near,ignition=[x,y],radius_m=60,
                    wind_from_deg=direction,wind_kmh=speed,moisture_pct=moisture,minutes=240,start_min=30)
        history = [dict(at_min=0,wind_from_deg=direction,wind_kmh=speed,moisture_pct=moisture)]
        if shift:
            at, turn, later_speed = shift
            case['shift'] = dict(at_min=at,wind_from_deg=(direction+turn)%360,wind_kmh=later_speed)
            history.append(dict(**case['shift'],moisture_pct=moisture))
        recipes.append(dict(zone=zone,profile=profile,case=case,history=history,native_case=True))
    # Refinements after the first screening: gentler winds rather than looser gates.
    refinements = [
        ('valico','vento_mite',27,8,(90,-25,24)),
        ('est','brezza_moderata',26,8,None),
        ('est','rotazione_mite',26,7,(75,-69,28)),
        ('valico','giro_tardivo',26,7,(105,-39,28)),
    ]
    for zone,profile,speed,moisture,shift in refinements:
        template = next(r for r in recipes if r['zone']==zone)
        case = dict(template['case'])
        case.update(name=f'giocabile_{zone}_{profile}',wind_kmh=speed,moisture_pct=moisture)
        case.pop('shift',None)
        direction = case['wind_from_deg']
        history = [dict(at_min=0,wind_from_deg=direction,wind_kmh=speed,moisture_pct=moisture)]
        if shift:
            at,turn,later_speed = shift
            case['shift'] = dict(at_min=at,wind_from_deg=(direction+turn)%360,wind_kmh=later_speed)
            history.append(dict(**case['shift'],moisture_pct=moisture))
        recipes.append(dict(zone=zone,profile=profile,case=case,history=history,native_case=True))
    dump(out/'recipes.json',dict(schema=2,mode='playable',recipes=recipes))
    print(f'{len(recipes)} combinazioni compatibili con il gioco in {out / "recipes.json"}',flush=True)


def generate_impacts(out):
    """Search different impact footprints, rather than relabeling weather."""
    starts = [
        ('nord_est','Castelvento',(5400,6100),45,100),
        ('nord','Fornaci',(5050,6200),30,315),
        ('costa_sud','San Rocco',(6800,3600),135,110),
        ('ovest_alto','Le Ghiande',(4000,4400),35,50),
        ('cresta_ovest','Le Ghiande',(4200,4200),60,45),
        ('ovest_doppio','Le Ghiande',(2900,4000),300,45),
    ]
    fuel = struct.unpack('<160000i',(ROOT/'data/scenarios/rocca_ventosa/fuel.i32').read_bytes())
    recipes = []
    for (zone,near,(x,y),direction,later),(strength,speed,moisture),turn in itertools.product(
            starts,[('mite',26,7),('teso',30,6),('forte',34,5)],(False,True)):
        assert fuel[int((8000-y)//20)*400+int(x//20)] > 0
        profile = strength+('_gira' if turn else '_costante')
        case = dict(name=f'impatti_{zone}_{profile}',near=near,ignition=[x,y],radius_m=60,
                    wind_from_deg=direction,wind_kmh=speed,moisture_pct=moisture,minutes=240,start_min=30)
        history = [dict(at_min=0,wind_from_deg=direction,wind_kmh=speed,moisture_pct=moisture)]
        if turn:
            case['shift'] = dict(at_min=90,wind_from_deg=later,wind_kmh=speed)
            history.append(dict(**case['shift'],moisture_pct=moisture))
        recipes.append(dict(zone=zone,profile=profile,case=case,history=history,native_case=True,impact_search=True))
    # Known routes crossing the ridge, with moderate weather variants.
    for zone,(x,y),speed,moisture,later,at in [
        ('est_valico',(5870,4750),28,7,94,75),
        ('est_valico',(5870,4750),30,6,50,75),
        ('valico_due_lati',(5590,4370),28,7,94,75),
        ('valico_due_lati',(5590,4370),30,6,50,75),
    ]:
        profile = f'{speed}_{moisture}'
        case = dict(name=f'impatti_{zone}_{profile}',near='Le Terrazze',ignition=[x,y],radius_m=60,
                    wind_from_deg=119,wind_kmh=speed,moisture_pct=moisture,minutes=240,start_min=30,
                    shift=dict(at_min=at,wind_from_deg=later,wind_kmh=speed))
        history = [dict(at_min=0,wind_from_deg=119,wind_kmh=speed,moisture_pct=moisture),
                   dict(**case['shift'],moisture_pct=moisture)]
        recipes.append(dict(zone=zone,profile=profile,case=case,history=history,native_case=True,impact_search=True))
    # Longer cross-town paths: the published game itself already allows 300 min.
    for zone,near,(x,y),direction,later,at,variants in [
        ('nord_ponte','Fornaci',(5050,6200),30,315,105,[(32,6),(36,5)]),
        ('nord_est_ponte','Castelvento',(5400,6100),45,315,90,[(32,6),(36,5)]),
        ('sud_promontorio','San Rocco',(6400,3300),165,120,90,[(30,6),(34,5)]),
        ('sud_est_ponte','San Rocco',(6600,3400),150,119,90,[(32,6),(36,5)]),
        ('cresta_lunga','Le Ghiande',(4200,4200),60,45,75,[(28,7),(32,6)]),
        ('ovest_lungo','Le Ghiande',(4500,4700),35,45,90,[(30,6),(34,5)]),
    ]:
        assert fuel[int((8000-y)//20)*400+int(x//20)] > 0
        for speed,moisture in variants:
            profile = f'{speed}_{moisture}'
            case = dict(name=f'impatti_{zone}_{profile}',near=near,ignition=[x,y],radius_m=60,
                        wind_from_deg=direction,wind_kmh=speed,moisture_pct=moisture,minutes=300,start_min=30,
                        shift=dict(at_min=at,wind_from_deg=later,wind_kmh=speed))
            history = [dict(at_min=0,wind_from_deg=direction,wind_kmh=speed,moisture_pct=moisture),
                       dict(**case['shift'],moisture_pct=moisture)]
            recipes.append(dict(zone=zone,profile=profile,case=case,history=history,native_case=True,impact_search=True))
    refinements = [
        ('nord_interno','Fornaci',(5050,6200),30,300,at,32,6,300,100) for at in (90,105,120)
    ] + [
        ('nord_interno_bis','Fornaci',(5300,5900),70,330,at,32,6,300,80) for at in (90,105,120)
    ] + [
        ('ovest_ridosso','Le Ghiande',(4000,4400),45,None,0,26,8,360,60),
        ('ovest_ridosso','Le Ghiande',(4000,4400),45,None,0,28,8,300,60),
        ('cresta_ridosso','Le Ghiande',(4200,4200),60,45,90,28,8,360,60),
        ('cresta_ridosso','Le Ghiande',(4200,4200),60,45,90,30,8,300,60),
        ('est_giro_lungo','Castelvento',(5870,4750),119,50,75,28,6,300,60),
        ('valico_costante_lungo','Le Terrazze',(5590,4370),119,None,0,28,7,300,60),
        ('valico_giro_lungo','Le Terrazze',(5590,4370),119,50,75,30,6,300,60),
    ]
    for zone,near,(x,y),direction,later,at,speed,moisture,minutes,radius in refinements:
        assert fuel[int((8000-y)//20)*400+int(x//20)] > 0
        profile = f'{speed}_{moisture}_{at}_{radius}'
        case = dict(name=f'impatti_{zone}_{profile}',near=near,ignition=[x,y],radius_m=radius,
                    wind_from_deg=direction,wind_kmh=speed,moisture_pct=moisture,minutes=minutes,start_min=30)
        history = [dict(at_min=0,wind_from_deg=direction,wind_kmh=speed,moisture_pct=moisture)]
        if later is not None:
            case['shift'] = dict(at_min=at,wind_from_deg=later,wind_kmh=speed)
            history.append(dict(**case['shift'],moisture_pct=moisture))
        recipes.append(dict(zone=zone,profile=profile,case=case,history=history,native_case=True,impact_search=True))
    dump(out/'recipes.json',dict(schema=3,mode='impacts',recipes=recipes))
    print(f'{len(recipes)} combinazioni per la varietà degli impatti',flush=True)


def impact_profile(runs,seeds,start_min):
    per_seed = []
    weights = {}
    frequency = {}
    for seed in seeds:
        districts = runs[seed,'none']['districts']
        meaningful = [d for d in districts if d['caught'] >= 10 or d['homes'] >= 2]
        houses = [d['name'] for d in districts if d['homes'] >= 2]
        mass = [d['caught']+2*d['homes'] for d in meaningful]
        secondary = 1-max(mass)/sum(mass) if mass else 0
        times = sorted(d['first_hit_min']-start_min for d in meaningful if d['first_hit_min'] is not None)
        gap = times[1]-times[0] if len(times)>1 else None
        for d in meaningful:
            name = d['name']
            frequency[name] = frequency.get(name,0)+1
            weights[name] = weights.get(name,0)+(d['caught']+2*d['homes'])/max(1,sum(mass))/len(seeds)
        defended = runs.get((seed,'adaptive'))
        per_seed.append(dict(seed=seed,boroughs=[d['name'] for d in meaningful],house_boroughs=houses,
                             all_boroughs=[d['name'] for d in districts if d['caught'] or d['homes']],
                             defended_impacts=[dict(name=d['name'],caught=d['caught'],homes=d['homes']) for d in defended['districts'] if d['caught'] or d['homes']] if defended else [],
                             secondary_share=round(secondary,4),first_gap_min=round(gap,1) if gap is not None else None,
                             impacts=[dict(name=d['name'],caught=d['caught'],homes=d['homes'],
                                           first_min=round(d['first_hit_min']-start_min,1) if d['first_hit_min'] is not None else None) for d in meaningful]))
    gaps = [e['first_gap_min'] for e in per_seed if e['first_gap_min'] is not None]
    median_gap = statistics.median(gaps) if gaps else None
    pattern = 'ravvicinati' if median_gap is not None and median_gap <= 20 else 'successivi'
    return dict(per_seed=per_seed,stable_boroughs=sorted(n for n,count in frequency.items() if count>=len(seeds)*.6),
                frequency=frequency,weights=weights,pattern=pattern,
                distributed_fraction=sum(len(e['boroughs'])>=2 and e['secondary_share']>=.15 for e in per_seed)/len(per_seed),
                minimum_boroughs=min(len(e['boroughs']) for e in per_seed),
                minimum_secondary_share=min(e['secondary_share'] for e in per_seed))


def search_impacts(args,out):
    """Spend full policy runs only on fires that really have distributed impacts."""
    recipes = json.loads((out/'recipes.json').read_text())['recipes']
    args.names,args.seeds,args.policies,args.delay = None,[2],['none'],0
    run(args,out)
    def shortlist(seeds):
        model = fingerprint()
        found = {}
        for path in (out/'runs').glob('*.json'):
            obj = json.loads(path.read_text());r = obj['result']
            if obj['model']==model and r['policy']=='none' and r.get('reaction_delay_min',0)==0:
                found[r['case'],r['seed']] = r
        chosen, evidence = [],[]
        for recipe in recipes:
            name = recipe['case']['name']
            if not all((name,s) in found for s in seeds):continue
            runs = {(s,'none'):found[name,s] for s in seeds}
            profile = impact_profile(runs,seeds,recipe['case']['start_min'])
            okay = profile['minimum_boroughs'] >= 2 and profile['minimum_secondary_share'] >= .15
            for seed in seeds:
                r = runs[seed,'none'];opening = r['opening']
                times = [d['first_hit_min']-recipe['case']['start_min'] for d in r['districts'] if d['first_hit_min'] is not None]
                okay &= bool(times) and 20<=min(times)<=150 and opening is not None and opening['caught']<=2 and opening['homes']==0
            evidence.append(dict(name=name,shortlisted=okay,impact=profile))
            if okay:chosen.append(name)
        dump(out/'baseline-screen.json',evidence)
        return chosen
    args.names = shortlist((2,))
    print(f'Prima selezione: {len(args.names)} incendi con impatti distribuiti',flush=True)
    if not args.names:return report(out)
    args.seeds = [1,3]
    run(args,out)
    args.names = shortlist((1,2,3))
    print(f'Selezione robusta degli impatti: {len(args.names)}',flush=True)
    if not args.names:return report(out)
    args.seeds,args.policies = [1,2,3],list(POLICIES)
    run(args,out)
    if any(r['eligible'] for r in analyse(out)):
        validate(args,out)
    else:
        report(out)


def focus_impacts(source,out):
    recipes = json.loads((source/'recipes.json').read_text())['recipes']
    names = {
        'impatti_cresta_ovest_teso_gira','impatti_cresta_ridosso_28_8_90_60',
        'impatti_ovest_lungo_30_6','impatti_valico_due_lati_30_6',
        'impatti_valico_giro_lungo_30_6_75_60','impatti_valico_costante_lungo_28_7_0_60',
        'impatti_est_giro_lungo_28_6_75_60',
    }
    focused = []
    for r in recipes:
        if r['case']['name'] not in names:continue
        clone = dict(r,planner='nuclei',case=dict(r['case']))
        clone['case']['name'] = r['case']['name'].replace('impatti_','impatti_nuclei_',1)
        focused.append(clone)
        if r['case']['name'] in ('impatti_valico_due_lati_30_6','impatti_valico_giro_lungo_30_6_75_60'):
            focused.append(r)
    dump(out/'recipes.json',dict(schema=3,mode='impacts',source=str(source),recipes=focused))
    print(f'{len(focused)} prove mirate sui nuclei abitati',flush=True)


def run(args, out):
    if os.environ.get('ROCCA_SCENARIO', 'rocca_ventosa') != 'rocca_ventosa':
        raise ValueError('La suite usa rocca_ventosa: rimuovere ROCCA_SCENARIO')
    recipes = json.loads((out / 'recipes.json').read_text())['recipes']
    if args.names:
        recipes = [r for r in recipes if r['case']['name'] in args.names]
        if not recipes:
            raise ValueError('Nessun nome di caso corrispondente')
    if args.limit:
        recipes = recipes[:args.limit]
    binary = ROOT / 'target/release/examples/scenario_suite'
    # Cargo rebuilds only the headless example; no graphical executable is launched.
    subprocess.run(['nice', '-n', '15', 'cargo', 'build', '--offline', '--release',
                    '-p', 'rocca', '--example', 'scenario_suite', '-j', '1'], cwd=ROOT, check=True)
    model = fingerprint()
    dump(out / 'provenance.json', dict(model=model, seeds=args.seeds, policies=args.policies))
    for recipe, seed, policy in itertools.product(recipes, args.seeds, args.policies):
        request = dict(case=recipe['case'], history=recipe['history'], seed=seed, policy=policy,
                       native_case=recipe.get('native_case',False),reaction_delay_min=args.delay)
        if recipe.get('planner'):
            request['planner'] = recipe['planner']
        key = hashlib.sha256((model + json.dumps(request, sort_keys=True)).encode()).hexdigest()
        dest = out / 'runs' / f'{key}.json'
        if dest.exists():
            continue
        print(f'{recipe["case"]["name"]} seme={seed} {policy}', flush=True)
        process = subprocess.run(['nice', '-n', '15', str(binary)], cwd=ROOT,
                                 input=json.dumps(request), text=True, capture_output=True, timeout=args.timeout)
        if process.returncode:
            raise RuntimeError(process.stderr)
        dump(dest, dict(key=key, model=model, recipe=recipe, request=request, result=json.loads(process.stdout)))
    if fingerprint() != model:
        raise RuntimeError('Sorgenti o dati cambiati durante le prove: rieseguire run per aggiornare i risultati.')
    report(out)


def analyse(out):
    model = fingerprint()
    recipes = json.loads((out / 'recipes.json').read_text())['recipes']
    rows = []
    for recipe in recipes:
        runs = {}
        delayed = {}
        for path in sorted((out / 'runs').glob('*.json')):
            obj = json.loads(path.read_text())
            if obj['model'] == model and obj['recipe'] == recipe:
                r = obj['result']
                delay = r.get('reaction_delay_min',0)
                if delay == 0:
                    runs[r['seed'], r['policy']] = r
                elif delay == 10 and r['policy'] == 'adaptive':
                    delayed[r['seed']] = r
        complete = all((seed, policy) in runs for seed in (1, 2, 3) for policy in POLICIES)
        if not complete:
            continue
        mean = lambda policy, metric: statistics.mean(runs[s, policy][metric] for s in (1, 2, 3))
        baseline = mean('none', 'caught')
        # Comparable loss scale across all candidates: people first, then homes,
        # then land; this is a search heuristic, not a replacement for game stars.
        loss = lambda p: mean(p, 'caught') + 2*mean(p, 'homes') + mean(p, 'ha')/10
        best = min(POLICIES[1:], key=loss)
        score = loss(best)
        impacted = sorted({d['name'] for s in (1,2,3) for d in runs[s,'none']['districts'] if d['caught'] or d['homes']})
        first = [d['first_hit_min']-recipe['case']['start_min'] for s in (1,2,3)
                 for d in runs[s,'none']['districts'] if d['first_hit_min'] is not None]
        trivial = all(loss(p) <= max(1, loss('none')*.1) for p in POLICIES[1:])
        opening_caught = statistics.mean((runs[s,'none']['opening'] or {}).get('caught',0) for s in (1,2,3))
        eligible = baseline >= 10 and mean('none','homes') >= 1 and not trivial and opening_caught <= baseline*.25
        row = dict(name=recipe['case']['name'], recipe=recipe, score=score,
                         eligible=eligible, opening_caught=round(opening_caught,1), best_policy=best, caught=round(baseline,1),
                         residual=round(mean(best,'caught'),1), homes=round(mean('none','homes'),1),
                         ha=round(mean('none','ha'),1), districts=impacted,
                         first_hit_after_report_min=round(min(first),1) if first else None,
                         spread=[runs[s,best]['caught'] for s in (1,2,3)],
                         policies={p:dict(caught=round(mean(p,'caught'),1), homes=round(mean(p,'homes'),1),
                                          ha=round(mean(p,'ha'),1), loss=round(loss(p),2)) for p in POLICIES})
        if recipe.get('native_case'):
            holdouts = all((s,p) in runs for s in (4,5) for p in POLICIES)
            seeds = (1,2,3,4,5) if holdouts else (1,2,3)
            row.update(playability(runs,recipe,seeds))
            row['policies'] = {p:{metric:round(statistics.mean(runs[s,p][metric] for s in seeds),2)
                                   for metric in ('caught','homes','ha','units_lost','moves','evacuations')} for p in POLICIES}
            row['impact'] = impact_profile(runs,seeds,recipe['case']['start_min'])
            if recipe.get('impact_search') and row['impact']['distributed_fraction'] < .8:
                row['eligible'] = False
                row['playability_rejections'].append('impatti distribuiti in meno dell’80% dei semi')
            row['playable'] = row['eligible']
            row['forgiving'] = None
            row['evaluated_seeds'] = list(seeds)
            row['validated'] = False
            if holdouts and all(s in delayed for s in seeds):
                stress_runs = dict(runs)
                stress_runs.update({(s,'adaptive'):delayed[s] for s in seeds})
                stress = playability(stress_runs,recipe,seeds)['per_seed']
                row['delay_test'] = stress
                row['validated'] = True
                row['forgiving'] = True
                for e in stress:
                    if e['people_saved'] < .6 or e['homes_saved'] < .1 or e['units_lost']:
                        row['eligible'] = False
                        row['forgiving'] = False
                        row['playability_rejections'].append(f'seme {e["seed"]}: troppo fragile con 10 min di ritardo negli ordini')
        rows.append(row)
    rows.sort(key=lambda r:(r['score'], r['name']))
    return rows


def validate(args,out):
    rows = analyse(out)
    names = [r['name'] for r in rows if r['eligible'] and (not args.names or r['name'] in args.names)]
    if not names:
        raise ValueError('Nessun candidato supera i filtri sui semi 1–3')
    args.names, args.seeds, args.policies, args.delay = names, [4,5], list(POLICIES), 0
    run(args,out)
    args.seeds, args.policies, args.delay = [1,2,3,4,5], ['adaptive'], 10
    run(args,out)
    report(out)


def playability(runs, recipe, seeds=(1,2,3)):
    """Conservative per-seed gates: one policy must work on every seed."""
    reasons = []
    evidence = []
    trivial = True
    for seed in seeds:
        baseline, defended, attack = (runs[seed,p] for p in ('none','adaptive','ignition'))
        first = [d['first_hit_min']-recipe['case']['start_min'] for d in baseline['districts']
                 if d['first_hit_min'] is not None]
        window = min(first) if first else None
        opening = baseline.get('opening')
        people_saved = 1-defended['caught']/max(1,baseline['caught'])
        homes_saved = 1-defended['homes']/max(1,baseline['homes'])
        ha_saved = 1-defended['ha']/max(1,baseline['ha'])
        if baseline['caught'] < 15 or (not recipe.get('impact_search') and baseline['homes'] < 2):
            reasons.append(f'seme {seed}: posta in gioco insufficiente')
        if opening is None or opening['caught'] > 2 or opening['homes'] > 0:
            reasons.append(f'seme {seed}: danni già presenti alla segnalazione')
        if window is None or not 20 <= window <= 150:
            reasons.append(f'seme {seed}: finestra intervento fuori 20–150 min')
        if people_saved < .8 or homes_saved < .25 or defended['units_lost'] > 0:
            reasons.append(f'seme {seed}: piano non abbastanza efficace/sicuro')
        if defended.get('moves',999) > 15 or defended.get('evacuations',999) > 4:
            reasons.append(f'seme {seed}: troppi ordini')
        trivial &= (attack['caught'] <= baseline['caught']*.1 and attack['homes'] <= baseline['homes']*.25
                    and attack['ha'] <= baseline['ha']*.5 and attack['units_lost'] == 0)
        evidence.append(dict(seed=seed,window_min=round(window,1) if window is not None else None,
                             baseline_caught=baseline['caught'],baseline_homes=baseline['homes'],baseline_ha=baseline['ha'],
                             caught=defended['caught'],homes=defended['homes'],ha=round(defended['ha'],1),
                             people_saved=round(people_saved,4),homes_saved=round(homes_saved,4),
                             ha_saved=round(ha_saved,4),units_lost=defended['units_lost'],
                             moves=defended.get('moves'),evacuations=defended.get('evacuations')))
    if trivial:
        reasons.append('tutte le squadre all’innesco risolve quasi tutto in ogni seme')
    difficulty = 100*(.5*(1-min(e['people_saved'] for e in evidence))
                      + .3*(1-min(e['homes_saved'] for e in evidence))
                      + .2*(1-min(e['ha_saved'] for e in evidence)))
    return dict(eligible=not reasons,playability_rejections=reasons,per_seed=evidence,
                caught=round(statistics.mean(e['baseline_caught'] for e in evidence),1),
                homes=round(statistics.mean(e['baseline_homes'] for e in evidence),1),
                ha=round(statistics.mean(e['baseline_ha'] for e in evidence),1),
                first_hit_after_report_min=min((e['window_min'] for e in evidence if e['window_min'] is not None),default=None),
                districts=sorted({d['name'] for seed in seeds for d in runs[seed,'none']['districts'] if d['caught'] or d['homes']}),
                score=round(difficulty,3),best_policy='adaptive',
                residual=round(statistics.mean(e['caught'] for e in evidence),1),
                spread=[e['caught'] for e in evidence],game_ready=True)


def select(rows):
    eligible = [r for r in rows if r['eligible']]
    if not eligible:
        return []
    if any(r['recipe'].get('impact_search') for r in eligible):
        # Actual borough footprints determine variety, not ignition labels.
        def signature(r):
            return tuple(r['impact']['stable_boroughs']),r['impact']['pattern']
        count = min(3,len({signature(r) for r in eligible}))
        groups = [g for g in itertools.combinations(eligible,count) if len({signature(r) for r in g})==count]
        def diversity(g):
            cover = len({n for r in g for n in r['impact']['stable_boroughs']})
            distance = sum(sum(abs(a['impact']['weights'].get(n,0)-b['impact']['weights'].get(n,0))
                               for n in set(a['impact']['weights'])|set(b['impact']['weights']))/2
                           for a,b in itertools.combinations(g,2))
            return 2*cover+distance-sum(r['score'] for r in g)/300
        return sorted(max(groups,key=diversity),key=lambda r:r['score'])
    # Require a real gap in measured loss; never invent three levels from ties.
    width = max(1, eligible[-1]['score']-eligible[0]['score'])
    gap = max(5, width * .1)
    for count in range(min(3, len(eligible)), 0, -1):
        choices = [group for group in itertools.combinations(eligible, count)
                   if all(b['score']-a['score'] >= gap for a,b in zip(group, group[1:]))]
        if not choices:
            continue
        def cost(group):
            targets = [eligible[0]['score'] + width*i/max(1,count-1) for i in range(count)]
            distance = sum(abs(r['score']-target)/width for r,target in zip(group,targets))
            zones = len({r['recipe']['zone'] for r in group})
            profiles = len({r['recipe']['profile'] for r in group})
            return distance + .25*(count-zones) + .15*(count-profiles)
        return list(min(choices, key=cost))
    return []



def variety_candidates(rows):
    """Keep strict robustness flags; separately expose playable harder cases."""
    families = {}
    for r in rows:
        if not (r.get('playable') and r.get('validated') and r['recipe'].get('impact_search')):
            continue
        signature = tuple(r['impact']['stable_boroughs']),r['impact']['pattern']
        old = families.get(signature)
        if old is None or (not r['eligible'],r['score']) < (not old['eligible'],old['score']):
            families[signature] = r
    proposals = select([dict(r,eligible=True) for r in families.values()])
    names = {r['name'] for r in proposals}
    return sorted([r for r in families.values() if r['name'] in names],key=lambda r:r['score'])


def report(out):
    rows = analyse(out)
    chosen = select(rows)
    variety = variety_candidates(rows)
    dump(out/'variety-candidates.json',dict(schema=1,candidates=variety))
    dump(out/'variety-cases.json',dict(cases=[r['recipe']['case'] for r in variety]))
    dump(out / 'analysis.json', rows)
    dump(out / 'selected.json', dict(schema=2, experimental=True, selected=chosen))
    native = [r for r in chosen if r.get('game_ready') and r.get('validated')]
    dump(out / 'game-cases.json', dict(cases=[r['recipe']['case'] for r in native]))
    # Complete configs for later installation, kept outside the live data tree.
    exports = [(r,'configs','plans') for r in native]+[(r,'review-configs','review-plans') for r in variety]
    for r,config_folder,plan_folder in exports:
        config = json.loads((ROOT/'data/scenarios/rocca_ventosa/game.json').read_text())
        names = {item['name'] for item in config['cases']}
        config['cases'] += [item['recipe']['case'] for item in (variety if config_folder=='review-configs' else native) if item['name'] not in names]
        config['game'] = r['name']
        dump(out/config_folder/r['name']/'game.json',config)
        model = fingerprint()
        for path in (out/'runs').glob('*.json'):
            record = json.loads(path.read_text())
            result = record['result']
            if record['model'] == model and record['recipe'] == r['recipe'] and result['policy'] == 'adaptive' and result.get('reaction_delay_min',0) == 0:
                orders = [dict(order,minutes_after_report=round(order['minute']-r['recipe']['case']['start_min'],1)) for order in result['orders']]
                dump(out/plan_folder/r['name']/f'seed-{result["seed"]}.json',dict(case=r['recipe']['case'],seed=result['seed'],orders=orders,
                     outcome={k:result[k] for k in ('caught','homes','ha','units_lost','moves','evacuations')}))


    native_mode = any(r.get('game_ready') for r in rows)
    lines = ['# Ricerca scenari — prova headless', '',
             ('Ricerca sui semi 1–3; conferma sui semi 4–5 e con 10 min di ritardo nei primi ordini. Medie sui semi indicati.' if native_mode else
              'Semi 1–3; medie. Difficoltà provvisoria dai danni residui della migliore delle tre politiche di prova.'),
             ('Indice di difficoltà dalle peggiori quote residue fra i semi, con il piano adattivo.' if native_mode else
              'Indice = persone colte + 2 × case colpite + ettari / 10. Non misura l’abilità di un giocatore.'),
             'Le storie di umidità sono valori del combustibile, imposti a gradini; non un modello di essiccamento.', '',
             '| Caso | Indice | Persone senza ordini | Persone residue con difesa | Case senza ordini | ha senza ordini | Prima minaccia dopo segnalazione (min) | Borghi |',
             '|---|---:|---:|---:|---:|---:|---:|---|']
    for r in rows:
        lines.append(f'| {r["name"]}{" ★" if r in chosen else ""} | {r["score"]:.1f} | {r["caught"]} | {r["residual"]} | {r["homes"]} | {r["ha"]} | {r["first_hit_after_report_min"]} | {", ".join(r["districts"])} |')
    lines += ['', f'{len(rows)} candidati completi; {len(chosen)} selezionati (★).', '']
    if variety:
        lines += ['## Combinazioni con impatti diversi', '',
                  'Tutte superano i criteri del piano ordinario su ogni seme (persone/case risparmiate e nessuna unità persa).',
                  'La robustezza al ritardo di 10 min è valutata separatamente: un caso meno permissivo non viene presentato come robusto.',
                  'game-cases.json conserva solo la selezione più permissiva. variety-cases.json e review-configs/ includono anche le varianti da rivedere.', '',
                  '| Caso | Borghi principali senza ordini | Borghi totali nei semi 1–5 | Persone/case/unità colpite col piano (massimi) | Ritardo 10 min: persone/case/unità (massimi) |',
                  '|---|---|---|---|---|']
        for r in variety:
            regular = '/'.join(str(max(e[k] for e in r['per_seed'])) for k in ('caught','homes','units_lost'))
            delayed = '/'.join(str(max(e[k] for e in r['delay_test'])) for k in ('caught','homes','units_lost'))
            counts = [len(e['all_boroughs']) for e in r['impact']['per_seed']]
            lines.append(f'| {r["name"]} | {", ".join(r["impact"]["stable_boroughs"])} | {counts} | {regular} | {delayed} |')
        lines += ['', 'I borghi principali hanno almeno 10 persone o 2 case colpite in almeno il 60% dei semi. Tutti i conteggi includono anche gli impatti minori.', '']

    if any(r['recipe'].get('impact_search') for r in rows):
        lines += ['Varietà misurata sugli impatti senza ordini: almeno 2 borghi in almeno l’80% dei semi, ciascuno con almeno 10 persone o 2 case colpite;',
                  'i borghi secondari devono pesare almeno il 15% di persone + 2×case. La selezione copre combinazioni di borghi e tempi diversi.',
                  'Per queste ricerche sono ammessi anche incendi che minacciano persone senza colpire case: il rapporto distingue sempre i due impatti.', '',
                  '| Scenario | Seme | Borghi colpiti (tutti / consistenti) | Impatti senza ordini: persone / case / primo minuto dopo segnalazione | Impatti con piano: persone / case |',
                  '|---|---:|---:|---|---|']
        for r in (variety or chosen):
            for e in r['impact']['per_seed']:
                before = '; '.join(f'{d["name"]}: {d["caught"]}/{d["homes"]}, +{d["first_min"]} min' for d in e['impacts'])
                after = '; '.join(f'{d["name"]}: {d["caught"]}/{d["homes"]}' for d in e['defended_impacts']) or 'nessuno'
                lines.append(f'| {r["name"]} | {e["seed"]} | {len(e["all_boroughs"])}/{len(e["boroughs"])} | {before} | {after} |')
        lines += ['']

    if any(r.get('game_ready') for r in rows):
        lines += ['Per i casi compatibili: indice dalle peggiori quote residue fra i semi, persone 50%, case 30%, ettari 20%.',
                  'Filtro per ogni seme: almeno 15 persone e 2 case senza ordini; apertura con massimo 2 persone colte e zero case;',
                  'prima minaccia a 20–150 min dalla segnalazione; piano adattivo salva almeno 80% persone e 25% case, zero unità perse;',
                  'massimo 15 spostamenti e 4 evacuazioni. Con 10 min di ritardo: almeno 60% persone e 10% case risparmiate, zero unità perse.',
                  'Sono soglie di ricerca, non nuove regole del gioco. Il fallimento di una politica non dimostra impossibilità del caso.', '']
        for r in rows:
            if r.get('playability_rejections'):
                lines.append(f'- Escluso `{r["name"]}`: '+ '; '.join(r['playability_rejections']))
        lines += ['', '| Candidato | Seme | Finestra (min) | Persone senza/con piano | Case senza/con piano | Unità perse | Ordini spostamento/evacuazione |',
                  '|---|---:|---:|---:|---:|---:|---:|']
        for r in chosen:
            for e in r.get('per_seed',[]):
                lines.append(f'| {r["name"]} | {e["seed"]} | {e["window_min"]} | {e["baseline_caught"]}/{e["caught"]} | {e["baseline_homes"]}/{e["homes"]} | {e["units_lost"]} | {e["moves"]}/{e["evacuations"]} |')
        lines += ['', 'game-cases.json esporta solo i selezionati confermati anche sui semi 4–5 e con ordini ritardati di 10 min.', '']
    for level, r in enumerate(chosen, 1):
        recipe = r['recipe']
        if r.get('game_ready'):
            lines += [f'Validazione `{r["name"]}`: semi {r["evaluated_seeds"]}, prova di ritardo {"completa" if r["validated"] else "da eseguire"}.']
            if r.get('delay_test'):
                lines += ['Persone colte con 10 min di ritardo: '+str([e['caught'] for e in r['delay_test']])+'.']
        lines += [f'{level}. **{r["name"]}** — innesco {recipe["case"]["ignition"]}; '
                  f'politica migliore: `{r["best_policy"]}`; persone residue nei semi valutati: {r["spread"]}.']
        for w in recipe['history']:
            lines.append(f'   - Da +{w["at_min"]} min: vento da {w["wind_from_deg"]}°, '
                         f'{w["wind_kmh"]} km/h; umidità combustibile {w["moisture_pct"]}%.')
    lines += ['', 'Nessun dato è stato installato nella demo e nessuna finestra è stata aperta.']
    if native_mode:
        lines += ['I casi selezionati usano direttamente il calendario meteo nativo di Game, senza estensioni al motore.',
                  'Umidità costante per caso e al massimo un cambio di vento; scenari esportati in game-cases.json.',
                  'La giocabilità è verificata con politiche headless; il divertimento e la leggibilità richiedono una prova umana.']
    else:
        lines += ['Il gioco pubblicato supporta un solo cambio di vento e umidità costante: le storie a più tappe richiedono integrazione.',
                  'Le politiche usano solo il fronte attuale, pause e ricarica del gioco.']
    lines += ['Un tempo negativo indica una minaccia precedente alla segnalazione. Nessuna dichiarazione di bilanciamento definitivo.']
    (out / 'report.md').write_text('\n'.join(lines)+'\n')
    print('\n'.join(lines), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('generate', 'run', 'validate', 'search-impacts', 'focus-impacts', 'report'))
    parser.add_argument('--out', type=Path, default=Path('results/scenario-suite'))
    parser.add_argument('--limit', type=int)
    parser.add_argument('--playable', action='store_true')
    parser.add_argument('--impacts', action='store_true')
    parser.add_argument('--source', type=Path, default=Path('results/scenario-impacts'))
    parser.add_argument('--names', nargs='+')
    parser.add_argument('--delay', type=int, default=0)
    parser.add_argument('--seeds', type=int, nargs='+', default=[1,2,3])
    parser.add_argument('--policies', nargs='+', choices=POLICIES, default=list(POLICIES))
    parser.add_argument('--timeout', type=int, default=240)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if args.command == 'generate': generate(out,args.playable,args.impacts)
    elif args.command == 'run': run(args, out)
    elif args.command == 'validate': validate(args,out)
    elif args.command == 'search-impacts': search_impacts(args,out)
    elif args.command == 'focus-impacts': focus_impacts(args.source.resolve(),out)
    else: report(out)

if __name__ == '__main__':
    main()
