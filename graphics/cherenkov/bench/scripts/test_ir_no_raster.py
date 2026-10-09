#!/usr/bin/env python3
"""Linux/lavapipe smoke test for the Ir gate's manifest-loaded layer."""
import importlib.util
import os
import pathlib
import resource
import signal
import subprocess
import tempfile
import unittest

SOURCE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location('ir_gate', SOURCE / 'ir_gate.py')
ir_gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ir_gate)


class NoRasterLayer(unittest.TestCase):
    def test_local_loader_dispatch(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = pathlib.Path(directory)
            library = ir_gate.install_layer(directory)
            probe = directory / 'probe'
            subprocess.run(['cc', '-O2', '-Wall', '-Wextra', '-Werror', str(SOURCE / 'ir_no_raster_test.c'),
                            '-ldl', '-o', str(probe)], check=True)
            symbols = subprocess.check_output(['nm', '-D', '--defined-only', str(library)], text=True)
            self.assertNotIn(' vkGetInstanceProcAddr\n', symbols)
            self.assertNotIn(' vkGetDeviceProcAddr\n', symbols)
            env = dict(os.environ, **ir_gate.layer_env(directory), XDG_RUNTIME_DIR=str(directory),
                       CHERENKOV_TEST_LAYER=str(library))
            subprocess.run([str(probe)], env=env, check=True)
            result = subprocess.run([str(probe), 'unregistered'], env=env, capture_output=True,
                                    preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_CORE, (0, 0)))
            self.assertEqual(result.returncode, -signal.SIGABRT)
            self.assertIn(b'vkDestroyInstance on unregistered instance', result.stderr)


if __name__ == '__main__':
    unittest.main()
