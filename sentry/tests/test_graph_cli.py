"""Real CLI processes: projection, transactional mutation and crash recovery.

No test transport or alternative graph parser participates in the application.
Fixtures are the existing GraphStore v1 format; JSON is checked with Python's parser.
"""
import concurrent.futures
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

CLI = Path(sys.argv.pop(1)).resolve()


def tsv(value):
    return value.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")


class GraphCLI(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="sentry-cli-")
        self.addCleanup(self.temp.cleanup)
        self.graph = Path(self.temp.name) / "graph.tsv"
        self.env = dict(os.environ, ARCHIE_SENTRY_GRAPH=str(self.graph))

    def run_cli(self, *args):
        return subprocess.run([str(CLI), *args], env=self.env, capture_output=True,
                              encoding="utf-8", timeout=30)

    def fixture(self, extra="", label="Owner"):
        self.graph.write_text("v\t1\nowner\towner\nsubject\towner\towner\t" + tsv(label) + "\n" + extra,
                              encoding="utf-8")

    def view(self, *args):
        result = self.run_cli("export-json", *args)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_projection_escapes_controls_and_preserves_utf8(self):
        label = 'Nelson "N" \\ 🌷\n\t\r\x01'
        self.fixture(label=label)
        view = self.view()
        self.assertEqual(view["schema_version"], 1)
        self.assertEqual(view["owner_id"], "owner")
        self.assertEqual(view["nodes"][0]["label"], label)
        self.assertEqual(view["nodes"][0]["degree"], 0)
        self.assertEqual(view["nodes"][0]["path_to_owner"], ["owner"])

    def test_sensitive_values_and_notes_are_redacted_unless_explicit(self):
        self.fixture("trait\towner\thome_address\tSECRET_ADDRESS\towner_declared\t-\tSECRET_NOTE\n")
        view = self.view()
        trait = view["nodes"][0]["traits"][0]
        self.assertIsNone(trait["value"])
        self.assertTrue(trait["redacted"])
        self.assertEqual(trait["source_note"], "")
        self.assertNotIn("SECRET", json.dumps(view))
        revealed = self.view("--all")["nodes"][0]["traits"][0]
        self.assertEqual(revealed["value"], "SECRET_ADDRESS")
        self.assertEqual(revealed["source_note"], "SECRET_NOTE")
        self.assertFalse(revealed["redacted"])

    def test_hypotheses_remain_visible_but_never_establish_reachability(self):
        self.fixture("subject\ta\tconsented_person\tAna\nsubject\tb\tconsented_person\tBeto\n"
                     "edge\towner\ta\tfriend\towner_declared\towner\tconfirmed\n"
                     "hyp\ta\tb\tfriend\tmodel_hypothesis\t\tguess\n")
        view = self.view()
        nodes = {n["id"]: n for n in view["nodes"]}
        self.assertEqual(nodes["a"]["degree"], 1)
        self.assertEqual(nodes["a"]["path_to_owner"], ["owner", "a"])
        self.assertIsNone(nodes["b"]["degree"])
        self.assertIsNone(nodes["b"]["path_to_owner"])
        self.assertEqual([e["standing"] for e in view["edges"]], ["asserted", "hypothesis"])

    def test_tags_keep_their_basis_and_never_promote_a_guess(self):
        self.fixture("trait\towner\twanda.tag\tfamily\towner_declared\t-\tself\n"
                     "trait\towner\twanda.tag\tpossible_friend\tmodel_inferred\t-\tmodel\n")
        node = self.view()["nodes"][0]
        self.assertEqual([t["basis"] for t in node["tags"]], ["owner_declared", "model_inferred"])
        self.assertEqual(node["traits"], [])

    def test_projection_applies_existing_load_refusals(self):
        self.fixture("subject\treporter\tpublic_figure\tReporter\n"
                     "trait\treporter\thome_address\tPRIVATE\tpublic_record\t-\tPRIVATE\n"
                     "edge\towner\treporter\tfamily\tpublic_record\t\tPRIVATE\n")
        result = self.run_cli("export-json")
        self.assertEqual(result.returncode, 0, result.stderr)
        view = json.loads(result.stdout)
        self.assertEqual(view["edges"], [])
        self.assertEqual(view["nodes"][1]["traits"], [])
        self.assertNotIn("PRIVATE", result.stdout + result.stderr)
        self.assertIn("refused", result.stderr)

    def test_partner_and_manager_are_explicit_and_backwards_compatible(self):
        self.fixture("subject\ta\tconsented_person\tAna\nsubject\tb\tpublic_figure\tBoss\n"
                     "edge\towner\ta\tpartner\towner_declared\towner\tdeclared\n"
                     "edge\towner\tb\tmanager\tpublic_record\t\tprofessional\n"
                     "edge\towner\ta\tfriend\towner_declared\towner\told\n")
        view = self.view()
        self.assertEqual([e["kind"] for e in view["edges"]], ["partner", "manager", "friend"])
        self.fixture("subject\ta\tconsented_person\tAna\n"
                     "edge\towner\ta\tpartner\tpublic_record\t\tforbidden\n")
        self.assertEqual(self.view()["edges"], [])

    def test_concurrent_cli_writers_retain_every_update(self):
        initialized = self.run_cli("init", "owner")
        self.assertEqual(initialized.returncode, 0, initialized.stderr)
        with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
            results = list(pool.map(lambda i: self.run_cli("add-person", f"p{i}", "consented_person"), range(32)))
        for result in results:
            self.assertEqual(result.returncode, 0, result.stderr)
        view = self.view()
        self.assertEqual({n["id"] for n in view["nodes"]}, {"owner"} | {f"p{i}" for i in range(32)})

    def test_init_does_not_overwrite_an_existing_graph(self):
        self.fixture(label="keep me")
        before = self.graph.read_bytes()
        result = self.run_cli("init", "replacement")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.graph.read_bytes(), before)

    def test_oversized_graph_is_rejected_without_output(self):
        self.fixture()
        with self.graph.open("ab") as out:
            out.write(b"#" + b"x" * (4 * 1024 * 1024))
        before = self.graph.stat().st_size
        result = self.run_cli("export-json")
        self.assertEqual(result.returncode, 65)
        self.assertEqual(result.stdout, "")
        self.assertIn("limit", result.stderr)
        self.assertEqual(self.graph.stat().st_size, before)

    def test_projection_rejects_invalid_utf8_without_partial_json(self):
        self.graph.write_bytes(b"v\t1\nowner\towner\nsubject\towner\towner\tbad\xff\n")
        result = self.run_cli("export-json")
        self.assertEqual(result.returncode, 65)
        self.assertEqual(result.stdout, "")

    def test_projection_rejects_ids_over_the_wire_limit(self):
        self.fixture("subject\t" + "x" * 193 + "\tconsented_person\tToo long\n")
        result = self.run_cli("export-json")
        self.assertEqual(result.returncode, 65)
        self.assertEqual(result.stdout, "")

    def test_json_expansion_is_bounded_and_never_partially_written(self):
        # Input fits the 4 MiB store budget but JSON control escaping exceeds 8 MiB.
        self.fixture("trait\towner\tnote\t" + "\x01" * (2 * 1024 * 1024) + "\towner_declared\t-\t\n")
        result = self.run_cli("export-json")
        self.assertEqual(result.returncode, 65)
        self.assertEqual(result.stdout, "")

    def test_process_death_releases_transaction_without_deleting_lock_file(self):
        self.fixture()
        helper = CLI.parent / ("sentry-lock-holder.exe" if os.name == "nt" else "sentry-lock-holder")
        self.assertTrue(helper.is_file(), "missing transaction test helper")
        holder = subprocess.Popen([str(helper), str(self.graph)], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(lambda: holder.poll() is None and holder.kill())
        self.assertEqual(holder.stdout.readline().strip(), "locked")
        writer = subprocess.Popen([str(CLI), "add-person", "after-crash", "consented_person"],
                                  env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(lambda: writer.poll() is None and writer.kill())
        # A complete transaction holds the writer, not only its final rename.
        with self.assertRaises(subprocess.TimeoutExpired):
            writer.wait(timeout=0.3)
        holder.kill()
        holder.communicate(timeout=10)
        _, err = writer.communicate(timeout=10)
        self.assertEqual(writer.returncode, 0, err)
        self.assertTrue(Path(str(self.graph) + ".lock").is_file())
        self.assertIn("after-crash", {n["id"] for n in self.view()["nodes"]})

    def test_readers_neither_wait_for_a_writer_nor_need_the_lock_file(self):
        self.fixture()
        helper = CLI.parent / ("sentry-lock-holder.exe" if os.name == "nt" else "sentry-lock-holder")
        holder = subprocess.Popen([str(helper), str(self.graph)], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(lambda: holder.poll() is None and holder.kill())
        self.assertEqual(holder.stdout.readline().strip(), "locked")
        # A transaction in progress does not hold readers back.
        for command in (["export-json"], ["show"], ["check"]):
            result = subprocess.run([str(CLI), *command], env=self.env, capture_output=True,
                                    text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
        holder.kill()
        holder.communicate(timeout=10)
        # Nor does a reader create the lock file next to a graph it only reads.
        Path(str(self.graph) + ".lock").unlink()
        self.assertEqual(self.run_cli("export-json").returncode, 0)
        self.assertFalse(Path(str(self.graph) + ".lock").exists())


if __name__ == "__main__":
    unittest.main()
