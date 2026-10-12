#!/usr/bin/env python3
"""Import downloaded SRTMGL1 V003 HGT tiles; no credentials/network needed here."""
import argparse, hashlib, json, pathlib, re, shutil
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--source', type=pathlib.Path, help='Directory containing unzipped SRTM HGT tiles')
p.add_argument('--output', type=pathlib.Path, default=pathlib.Path('data/generated/terrain/regional'))
p.add_argument('--verify-only', action='store_true')
a = p.parse_args()
required = [f'N{lat:02d}W{west:03d}' for lat in (37,38) for west in (78,77,76)]
if a.verify_only:
    manifest = json.loads((a.output/'manifest.json').read_text())
    assert manifest['version'] == 1 and manifest['vertical_datum'] == 'EGM96'
    assert sorted(x['file'][:-4] for x in manifest['tiles']) == sorted(required)
    for tile in manifest['tiles']:
        data = (a.output/tile['file']).read_bytes()
        assert len(data) == 3601*3601*2 and hashlib.sha256(data).hexdigest() == tile['sha256'], tile['file']
    print('Regional terrain verified')
else:
    if a.source is None: p.error('--source is required unless --verify-only')
    if a.output.exists(): p.error('Output already exists; preserve it and select a fresh output directory')
    files=[]
    for name in required:
        matches=list(a.source.glob(name+'*.hgt'))
        if len(matches)!=1: p.error(f'Need exactly one unzipped {name} SRTMGL1 V003 HGT tile')
        data=matches[0].read_bytes()
        if len(data)!=3601*3601*2: p.error(f'Invalid SRTMGL1 dimensions: {matches[0]}')
        files.append((name,matches[0],hashlib.sha256(data).hexdigest()))
    a.output.mkdir(parents=True)
    tiles=[]
    for name,path,digest in files:
        dest=name+'.hgt'; shutil.copyfile(path,a.output/dest)
        tiles.append(dict(file=dest,latitude=int(name[1:3]),longitude=-int(name[4:7]),samples=3601,sha256=digest))
    manifest=dict(version=1,source='NASA SRTMGL1 V003; DOI 10.5067/MEaSUREs/SRTM/SRTMGL1.003',vertical_datum='EGM96',tiles=tiles)
    (a.output/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print(f'Imported {len(tiles)} tiles to {a.output}')
