from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch
from archie_wanda.contracts import TrainConfig, ResourceError
from archie_wanda.resources import run_guarded


class OwnedProcessGuardTests(unittest.TestCase):
    def test_resource_pressure_stops_only_the_owned_child(self):
        with tempfile.TemporaryDirectory() as directory:
            assert Path(directory).resolve().parent==Path(tempfile.gettempdir()).resolve()
            with (Path(directory)/'log').open('w') as log:
                with patch('archie_wanda.resources.preflight',side_effect=[object(),ResourceError('low disk')]):
                    with self.assertRaises(ResourceError):
                        run_guarded([sys.executable,'-c','import time; time.sleep(20)'],
                                    TrainConfig(model_path='/unused'),None,log,timeout=5,poll_seconds=.01)

    def test_owned_child_exit_and_stdout_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            assert Path(directory).resolve().parent==Path(tempfile.gettempdir()).resolve()
            with (Path(directory)/'log').open('w') as log:
                with patch('archie_wanda.resources.preflight',return_value=object()):
                    result=run_guarded([sys.executable,'-c','print("owned-result")'],
                                       TrainConfig(model_path='/unused'),None,log,timeout=5,poll_seconds=.01)
            self.assertEqual(result.returncode,0)
            self.assertEqual(result.stdout.strip(),'owned-result')
