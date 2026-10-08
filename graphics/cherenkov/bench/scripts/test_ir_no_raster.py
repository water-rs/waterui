#!/usr/bin/env python3
"""Linux/lavapipe smoke test for the Ir gate's manifest-loaded layer."""
import json
import os
import pathlib
import resource
import signal
import subprocess
import tempfile
import unittest


class NoRasterLayer(unittest.TestCase):
    def test_local_loader_dispatch(self):
        source = pathlib.Path(__file__).resolve().parent
        with tempfile.TemporaryDirectory() as directory:
            directory = pathlib.Path(directory)
            library = directory / 'layer.so'
            probe = directory / 'probe'
            flags = ['cc', '-O2', '-Wall', '-Wextra', '-Werror']
            subprocess.run(flags + ['-shared', '-fPIC', str(source / 'ir_no_raster.c'),
                                   '-o', str(library)], check=True)
            subprocess.run(flags + [str(source / 'ir_no_raster_test.c'), '-ldl',
                                   '-o', str(probe)], check=True)
            symbols = subprocess.check_output(['nm', '-D', '--defined-only', str(library)], text=True)
            self.assertNotIn(' vkGetInstanceProcAddr\n', symbols)
            self.assertNotIn(' vkGetDeviceProcAddr\n', symbols)
            (directory / 'layer.json').write_text(json.dumps({
                'file_format_version': '1.0.0',
                'layer': {'name': 'VK_LAYER_CHERENKOV_no_raster', 'type': 'GLOBAL',
                          'library_path': str(library), 'api_version': '1.3.0',
                          'implementation_version': '1', 'description': 'Ir gate test',
                          'functions': {'vkGetInstanceProcAddr': 'cherenkovGetInstanceProcAddr',
                                        'vkGetDeviceProcAddr': 'cherenkovGetDeviceProcAddr'}}}))
            env = dict(os.environ, VK_LAYER_PATH=str(directory),
                       VK_INSTANCE_LAYERS='VK_LAYER_CHERENKOV_no_raster', NODEVICE_SELECT='1',
                       VK_ICD_FILENAMES='/usr/share/vulkan/icd.d/lvp_icd.x86_64.json',
                       XDG_RUNTIME_DIR=str(directory), CHERENKOV_TEST_LAYER=str(library))
            subprocess.run([str(probe)], env=env, check=True)
            result = subprocess.run([str(probe), 'unregistered'], env=env, capture_output=True,
                                    preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_CORE, (0, 0)))
            self.assertEqual(result.returncode, -signal.SIGABRT)
            self.assertIn(b'vkDestroyInstance on unregistered instance', result.stderr)


if __name__ == '__main__':
    unittest.main()
