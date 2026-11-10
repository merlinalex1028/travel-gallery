"""Check QA namespace discovery/deletion and linked-ancestor protection in a tiny fake home."""
from pathlib import Path
import importlib.util
import json
import sys
import tempfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('cleanup', Path(__file__).with_name('cleanup-test-artifacts.py'))
cleanup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cleanup)

with tempfile.TemporaryDirectory(prefix='travel-gallery-cleanup-check-') as temporary:
    home = Path(temporary)
    library = home/'Library'
    container = library/'Containers/com.travelgallery.offlineaudit/Data/Library'
    protected = [library/'Application Support/com.travelgallery.desktop/catalog.sqlite',
                 library/'Caches/com.travelgallery.desktop/preview',
                 library/'Preferences/com.travelgallery.desktop.plist', home/'Photos/original.png']
    for path in protected:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b'protected')
    expected = [container/'Application Support/com.travelgallery.desktop',
                container/'Caches/com.travelgallery.desktop',
                container/'WebKit/com.travelgallery.desktop',
                container/'HTTPStorages/com.travelgallery.desktop',
                container/'Application Support/com.travelgallery.offlineaudit',
                library/'Application Support/com.travelgallery.benchmark',
                library/'Application Support/com.travelgallery.development',
                library/'Caches/com.travelgallery.development']
    for path in expected:
        path.mkdir(parents=True)
        (path/'owned').write_bytes(b'QA')
    for path in (container/'Preferences/com.travelgallery.desktop.plist',
                 container/'Preferences/com.travelgallery.offlineaudit.plist',
                 library/'Preferences/com.travelgallery.development.plist'):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b'QA preference')
        expected.append(path)
    link = library/'Caches/com.travelgallery.offlineaudit'
    link.symlink_to(home/'Photos', target_is_directory=True)
    expected.append(link)
    targets = list(cleanup.qa_data_targets(home))
    assert set(targets) == set(expected)
    assert sum(cleanup.measure(path)['files'] for path in targets) == 12
    for path in targets:
        cleanup.remove_target(path)
    assert all(path.read_bytes() == b'protected' for path in protected)
    assert not list(cleanup.qa_data_targets(home))
    parent = container/'Application Support'
    parent.rmdir()
    parent.symlink_to(library/'Application Support', target_is_directory=True)
    try:
        list(cleanup.qa_data_targets(home))
    except RuntimeError as error:
        assert 'linked QA ancestor' in str(error)
    else:
        raise AssertionError('Linked ancestor could expose the normal catalog to cleanup')
    assert all(path.read_bytes() == b'protected' for path in protected)

print(json.dumps(dict(passed=True, discoveredAndRemovedTargets=12, normalCatalogAndCachePreserved=True,
                     originalPhotoPreserved=True, targetSymlinkUnlinkedOnly=True, linkedAncestorRejected=True,
                     temporaryDirectoryRemoved=not Path(temporary).exists())))
