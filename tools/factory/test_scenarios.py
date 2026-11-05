"""Checks for selection and cache provenance, independent of expensive fire runs."""
import importlib.util
import json
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('suite', Path(__file__).with_name('scenarios.py'))
suite = importlib.util.module_from_spec(spec)
spec.loader.exec_module(suite)


def row(name, score, zone='est', profile='regolare', eligible=True):
    return dict(name=name, score=score, eligible=eligible, recipe=dict(zone=zone, profile=profile))


class SelectionTests(unittest.TestCase):
    def test_progression_covers_range_and_variety(self):
        rows = [row('a',10,'ovest'), row('b',45,'est'), row('c',50,'valico','rotazione'), row('d',90,'est','secco')]
        selected = suite.select(rows)
        self.assertEqual([r['name'] for r in selected], ['a','c','d'])

    def test_equal_scores_do_not_become_three_levels(self):
        self.assertEqual(len(suite.select([row('a',10),row('b',10),row('c',12)])), 1)
        self.assertEqual(suite.select([row('a',50,eligible=False)]), [])

    def test_partial_or_stale_runs_are_excluded(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            recipe = dict(case=dict(name='a'))
            suite.dump(out/'recipes.json',dict(recipes=[recipe]))
            for seed in (1,2,3):
                for policy in suite.POLICIES:
                    suite.dump(out/'runs'/f'{seed}-{policy}.json',dict(model='old',recipe=recipe,
                               result=dict(seed=seed,policy=policy)))
            with patch.object(suite,'fingerprint',return_value='new'):
                self.assertEqual(suite.analyse(out), [])
            suite.dump(out/'runs'/'1-none.json',dict(model='new',recipe=recipe,result=dict(seed=1,policy='none')))
            with patch.object(suite,'fingerprint',return_value='new'):
                self.assertEqual(suite.analyse(out), [])


class ProvenanceTests(unittest.TestCase):
    def test_display_and_feedback_edits_do_not_invalidate_the_model(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for crate in ('rocca','fire','abm','scenario','behavior','datafs'):
                (root/'crates'/crate/'src').mkdir(parents=True)
                (root/'crates'/crate/'Cargo.toml').write_text('manifest')
                (root/'crates'/crate/'src/lib.rs').write_text('model')
            for name in ('crates/rocca/examples/scenario_suite.rs','data/fuels_eu12.json','Cargo.lock','Cargo.toml','.cargo/config.toml'):
                path = root/name
                path.parent.mkdir(parents=True,exist_ok=True)
                path.write_text('input')
            with patch.object(suite,'ROOT',root):
                before = suite.fingerprint()
                (root/'crates/rocca/src/score.rs').write_text('new feedback')
                (root/'crates/game/src').mkdir(parents=True)
                (root/'crates/game/src/main.rs').write_text('new window')
                self.assertEqual(before,suite.fingerprint())
                (root/'crates/fire/src/lib.rs').write_text('changed fire physics')
                self.assertNotEqual(before,suite.fingerprint())


class RecipeTests(unittest.TestCase):
    def test_generator_is_reproducible_and_has_geographic_and_weather_variety(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            suite.generate(out)
            original = (out/'recipes.json').read_bytes()
            suite.generate(out)
            self.assertEqual(original, (out/'recipes.json').read_bytes())
            recipes = json.loads(original)['recipes']
            self.assertEqual(len({r['zone'] for r in recipes}), 3)
            self.assertEqual(len({r['profile'] for r in recipes}), 4)
            self.assertTrue(all(r['history'][0]['at_min'] == 0 for r in recipes))

    @unittest.skipUnless((suite.ROOT/'target/release/examples/scenario_suite').exists(), 'build runner first')
    def test_runner_rejects_unordered_weather_before_simulating(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            suite.generate(out)
            recipe = json.loads((out/'recipes.json').read_text())['recipes'][0]
            recipe['history'][1]['at_min'] = 0
            result = subprocess.run([str(suite.ROOT/'target/release/examples/scenario_suite')],
                         input=json.dumps(dict(case=recipe['case'], history=recipe['history'], seed=1, policy='none')),
                         text=True, capture_output=True, timeout=10, cwd=suite.ROOT)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('invalid weather time', result.stderr)


class PlayabilityTests(unittest.TestCase):
    def example_runs(self):
        runs = {}
        for seed in (1,2,3):
            for policy in suite.POLICIES:
                defended = policy == 'adaptive'
                runs[seed,policy] = dict(caught=5 if defended else 100,homes=4 if defended else 20,
                    ha=50 if defended else 100,units_lost=0,moves=10,evacuations=3,
                    opening=dict(caught=0,homes=0),districts=[dict(name='Borgo',caught=100,homes=20,first_hit_min=90)])
        return runs

    def test_one_bad_seed_is_not_hidden_by_the_mean(self):
        runs = self.example_runs()
        recipe = dict(case=dict(start_min=30))
        self.assertTrue(suite.playability(runs,recipe)['eligible'])
        runs[3,'adaptive']['caught'] = 50
        self.assertFalse(suite.playability(runs,recipe)['eligible'])

    def test_unavoidable_opening_damage_and_lost_units_are_rejected(self):
        for change in ('opening','unit','window'):
            runs = self.example_runs()
            if change == 'opening': runs[2,'none']['opening']['caught'] = 10
            if change == 'unit': runs[2,'adaptive']['units_lost'] = 1
            if change == 'window': runs[2,'none']['districts'][0]['first_hit_min'] = 35
            self.assertFalse(suite.playability(runs,dict(case=dict(start_min=30)))['eligible'])

    def test_playable_recipes_use_constant_moisture_and_at_most_one_shift(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            suite.generate(out,playable=True)
            for r in json.loads((out/'recipes.json').read_text())['recipes']:
                self.assertTrue(r['native_case'])
                self.assertLessEqual(len(r['history']),2)
                self.assertEqual(len({w['moisture_pct'] for w in r['history']}),1)

    def test_extra_seeds_and_delayed_orders_can_reject_a_training_success(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            recipe = dict(native_case=True,zone='est',profile='prova',case=dict(name='prova',start_min=30))
            suite.dump(out/'recipes.json',dict(recipes=[recipe]))
            for seed in (1,2,3,4,5):
                for policy in suite.POLICIES:
                    result = dict(self.example_runs()[1,policy],seed=seed,policy=policy)
                    suite.dump(out/'runs'/f'{seed}-{policy}.json',dict(model='m',recipe=recipe,result=result))
                delayed = dict(self.example_runs()[1,'adaptive'],seed=seed,policy='adaptive',reaction_delay_min=10)
                if seed == 5: delayed['caught'] = 50
                suite.dump(out/'runs'/f'{seed}-delayed.json',dict(model='m',recipe=recipe,result=delayed))
            with patch.object(suite,'fingerprint',return_value='m'):
                row = suite.analyse(out)[0]
            self.assertTrue(row['validated'])
            self.assertFalse(row['eligible'])
            self.assertEqual(row['evaluated_seeds'],[1,2,3,4,5])
            self.assertIn('seme 5: troppo fragile',row['playability_rejections'][0])

    @unittest.skipUnless((suite.ROOT/'target/release/examples/scenario_suite').exists(), 'build runner first')
    def test_native_calendar_and_external_replay_agree(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            suite.generate(out,playable=True)
            recipe = json.loads((out/'recipes.json').read_text())['recipes'][2]
            recipe['case']['minutes'] = 90
            results = []
            for native in (True,False):
                result = subprocess.run([str(suite.ROOT/'target/release/examples/scenario_suite')],
                    input=json.dumps(dict(case=recipe['case'],history=recipe['history'],seed=2,
                                          policy='none',native_case=native)),
                    text=True,capture_output=True,timeout=30,cwd=suite.ROOT,check=True)
                record = json.loads(result.stdout)
                record.pop('native_case')
                results.append(record)
            self.assertEqual(results[0],results[1])


class ImpactTests(unittest.TestCase):
    def test_different_single_boroughs_do_not_count_as_a_multi_borough_fire(self):
        runs = {(s,'none'):dict(districts=[dict(name=name,caught=40,homes=5,first_hit_min=90)])
                for s,name in enumerate(('A','B','C'),1)}
        profile = suite.impact_profile(runs,(1,2,3),30)
        self.assertEqual(profile['minimum_boroughs'],1)
        self.assertEqual(profile['distributed_fraction'],0)

    def test_impact_reliability_is_visible_per_seed(self):
        runs = {}
        for seed in (1,2,3,4,5):
            runs[seed,'none'] = dict(districts=[
                dict(name='A',caught=60,homes=10,first_hit_min=90),
                dict(name='B',caught=30 if seed!=5 else 0,homes=5 if seed!=5 else 0,first_hit_min=150 if seed!=5 else None)])
        profile = suite.impact_profile(runs,(1,2,3,4,5),30)
        self.assertEqual(profile['distributed_fraction'],.8)
        self.assertEqual(profile['frequency']['B'],4)
        self.assertEqual(profile['stable_boroughs'],['A','B'])
        self.assertEqual(profile['minimum_boroughs'],1)

    def test_variety_uses_impact_footprints_not_ignition_labels(self):
        rows = []
        for name,boroughs in [('a',['A','B']),('b',['A','B']),('c',['C','D'])]:
            r = row(name,10)
            r['recipe']['impact_search'] = True
            r['impact'] = dict(stable_boroughs=boroughs,pattern='successivi',weights={n:.5 for n in boroughs})
            rows.append(r)
        selected = suite.select(rows)
        self.assertEqual(len(selected),2)
        self.assertIn('c',[r['name'] for r in selected])

    def test_impact_recipes_are_native_and_burnable(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            suite.generate(out,impacts=True)
            recipes = json.loads((out/'recipes.json').read_text())['recipes']
            self.assertEqual(len({r['case']['name'] for r in recipes}),len(recipes))
            self.assertTrue(all(r['native_case'] and r['impact_search'] for r in recipes))
            self.assertTrue(all(len(r['history'])<=2 for r in recipes))


class NucleusPlannerTests(unittest.TestCase):
    @unittest.skipUnless((suite.ROOT/'target/release/examples/scenario_suite').exists(), 'build runner first')
    def test_plan_respects_order_budget_and_exports_actual_actions(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            suite.generate(out,impacts=True)
            recipe = next(r for r in json.loads((out/'recipes.json').read_text())['recipes']
                          if r['case']['name']=='impatti_cresta_ovest_teso_gira')
            result = subprocess.run([str(suite.ROOT/'target/release/examples/scenario_suite')],
                input=json.dumps(dict(case=recipe['case'],history=recipe['history'],seed=2,
                                      policy='adaptive',native_case=True,planner='nuclei')),
                text=True,capture_output=True,timeout=30,cwd=suite.ROOT,check=True)
            r = json.loads(result.stdout)
            self.assertEqual(r['planner'],'nuclei')
            self.assertLessEqual(r['moves'],15)
            self.assertLessEqual(r['evacuations'],4)
            self.assertEqual(r['moves'],sum('team' in o for o in r['orders']))
            self.assertEqual(r['evacuations'],sum('evacuate' in o for o in r['orders']))
            self.assertTrue(all(o['minute']>=recipe['case']['start_min'] for o in r['orders']))


class VarietyReviewTests(unittest.TestCase):
    def candidate(self,name,boroughs,score,forgiving):
        r = row(name,score,eligible=forgiving)
        r.update(playable=True,validated=True,forgiving=forgiving)
        r['recipe']['impact_search'] = True
        r['impact'] = dict(stable_boroughs=boroughs,pattern='successivi',weights={n:1/len(boroughs) for n in boroughs})
        return r

    def test_review_keeps_robustness_flags_and_prefers_forgiving_case_per_family(self):
        fragile = self.candidate('fragile',['A','B'],1,False)
        robust = self.candidate('robust',['A','B'],20,True)
        broad = self.candidate('broad',['A','B','C','D'],5,False)
        proposals = suite.variety_candidates([fragile,robust,broad])
        self.assertEqual({r['name'] for r in proposals},{'robust','broad'})
        self.assertFalse(broad['eligible'])
        self.assertFalse(broad['forgiving'])
        self.assertFalse(fragile['eligible'])


if __name__ == '__main__':
    unittest.main()
