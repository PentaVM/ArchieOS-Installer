"""CPU-only checks for the diagnostic sharing protocol; no live controls/GPU calls."""
from pathlib import Path
import signal
import tempfile
import unittest
from unittest.mock import patch
import high_semantic_cabina as run


class HoldProtocolTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory()
        self.root=Path(self.temp.name)
        self.hold=self.root/'hold'
        self.state={'pid':150,'boot_id':'test-boot','state':'paused'}

    def tearDown(self): self.temp.cleanup()

    def context(self):
        return run.OwnedHold(self.hold,self.root/'own.lock',lambda:dict(self.state))

    def test_preexisting_hold_is_preserved(self):
        self.hold.write_bytes(b'belongs to someone else')
        with self.assertRaises(FileExistsError):
            with self.context(): self.fail('preexisting hold admitted')
        self.assertEqual(self.hold.read_bytes(),b'belongs to someone else')

    def test_changed_hold_ownership_is_never_removed(self):
        with self.assertRaises(RuntimeError):
            with self.context(): self.hold.write_bytes(b'replacement owner')
        self.assertEqual(self.hold.read_bytes(),b'replacement owner')

    def test_changed_worker_identity_and_unpaused_state_refuse(self):
        for field,value in (('pid',151),('boot_id','new-boot'),('state','running')):
            with self.context() as hold:
                self.state[field]=value
                with self.assertRaises(RuntimeError): hold.assert_paused()
            self.state={'pid':150,'boot_id':'test-boot','state':'paused'}
        self.assertFalse(self.hold.exists())

    def test_thermal_or_other_error_terminates_and_waits_before_releasing_hold(self):
        for error in (RuntimeError('thermal stop'),KeyboardInterrupt()):
            events=[]
            owner=self
            class Child:
                pid=4242
                alive=True
                def poll(self): return None if self.alive else -15
                def wait(self,timeout):
                    owner.assertTrue(owner.hold.exists())
                    events.append('wait'); self.alive=False
            child=Child()
            def kill(group, sig):
                self.assertEqual(group,child.pid)
                self.assertEqual(sig,signal.SIGTERM)
                self.assertTrue(self.hold.exists())
                events.append('terminate')
            with patch.object(run.os,'killpg',side_effect=kill):
                with self.assertRaises(type(error)):
                    with self.context() as hold:
                        hold.child=child
                        raise error
            self.assertFalse(child.alive)
            self.assertFalse(self.hold.exists())
            self.assertEqual(events,['terminate','wait'])

    def test_failed_owned_child_shutdown_keeps_hold(self):
        class Child:
            pid=4242
            def poll(self): return None
            def wait(self,timeout): raise run.subprocess.TimeoutExpired('owned',timeout)
        with patch.object(run.os,'killpg'):
            with self.assertRaises(run.subprocess.TimeoutExpired):
                with self.context() as hold:
                    hold.child=Child()
                    raise RuntimeError('stop')
        self.assertTrue(self.hold.exists())


if __name__=='__main__': unittest.main()
