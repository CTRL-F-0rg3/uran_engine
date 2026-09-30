"""Autoryzuje klipy animacji na szkielecie z pliku GLB i zapisuje je
we własnym formacie binarnym (`.urananm`).

Dlaczego własny format, a nie ponowny eksport do glTF:

  * oryginalny GLB ma 22 osadzone tekstury, 11 materialow i dokladne
    wagi skinningu — ponowny eksport z Blendera moglby je znieksztalcic;
  * animacje sa jedynym brakujacym elementem, wiec trzymamy je osobno
    od geometrii;
  * wlasny format to ~40 linii kodu do odczytu w Rustcie, bez zaleznosci
    serde_json (sieć w tym środowisku jest zablokowana).

Format (wielkoscopolowy, kolejne pola bez paddingu):

    magic     8 B   b"URANANM1"
    bones     u32 + (u16 len + utf8) * bones
    clips     u32
    clip      u16 len + utf8, f32 duration, f32 fps, u32 tracks
    track     u32 bone, u32 nT + nT*(f32 t, 3*f32),
                     u32 nR + nR*(f32 t, 4*f32),
                     u32 nS + nS*(f32 t, 3*f32)

Uruchomienie:
    blender -b --factory-startup --python tools/author_anim.py -- \
        assets/theresa.glb out.urananm
"""
import json
import math
import struct
import sys
from pathlib import Path

import bpy
from mathutils import Matrix, Quaternion, Vector

FPS = 30.0
TAU = math.pi * 2.0


def read_gltf_skeleton(path):
    """Wczytuje JSON z GLB — wystarcza nam lista jointow skinu."""
    data = path.read_bytes()
    magic, _, length = struct.unpack('<III', data[:12])
    assert magic == 0x46546C67, 'nie jest GLB'
    off, chunks = 12, []
    while off < length:
        clen, ctype = struct.unpack('<II', data[off:off + 8])
        chunks.append((ctype, clen, off + 8))
        off += 8 + clen
    js, jl = chunks[0][2], chunks[0][1]
    return json.loads(data[js:js + jl].decode('utf-8'))


def import_glb(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(path), import_pack_images=True)
    for ob in bpy.data.objects:
        if ob.type == 'ARMATURE':
            return ob
    raise RuntimeError('brak armatury po imporcie')


def find_bone(arm, *keys):
    """Szuka kosci po podzbiorze nazwy; zwraca obiekt lub None."""
    for pb in arm.pose.bones:
        if all(k in pb.name for k in keys):
            return pb
    return None


# ----------------------------------------------------------- matrix helpers
# Pozycja spoczynkowa tego modelu to T-pose, a wszystkie wezly szkieletu
# maja zerowa rotacje (R=None w glTF) — osie wynikaja wylacznie z lancucha
# translacji. Dzieki temu obracanie „w przestrzeni swiata" przez macierz
# pozy jest jednoznaczne i nie wymaga recznego liczenia osi kazdej kosci.
def rot_world(pb, axis, angle):
    """Obraca kosc wokol jej glowej osi podanej w przestrzeni swiata."""
    if abs(angle) < 1e-9:
        return
    q = Quaternion(Vector(axis).normalized(), angle)
    m = pb.matrix.copy()
    p = m.translation
    pb.matrix = (Matrix.Translation(p) @ q.to_matrix().to_4x4() @
                 Matrix.Translation(-p) @ m)


def trans_world(pb, delta):
    """Przesuwa kosc w przestrzeni swiatu (delta w jednostkach modelu)."""
    d = Vector(delta)
    if d.length < 1e-9:
        return
    pb.matrix = Matrix.Translation(d) @ pb.matrix.copy()


def aim_dir(pb, target):
    """Ustawia kosc tak, by jej os Y wskazywala `target` w przestrzeni
    swiata — uzywane do opuszczenia rak z pozy T do stojacej."""
    m = pb.matrix.copy()
    cur = (m.to_3x3() @ Vector((0.0, 1.0, 0.0))).normalized()
    q = cur.rotation_difference(Vector(target).normalized())
    pb.matrix = (Matrix.Translation(m.translation) @
                 q.to_matrix().to_4x4() @ m.to_3x3().to_4x4())


def spin(pb, angle, axis='Z'):
    """Dodaje obrot do biezacej macierzy kosci."""
    pb.matrix = Matrix.Rotation(angle, 4, axis) @ pb.matrix.copy()


def sync():
    bpy.context.view_layer.update()


class Rig:
    """Wiązanka nazw kości z obiektami pose, żeby kod animacji był czytelny."""

    def __init__(self, arm):
        self.arm = arm
        self.hips = find_bone(arm, 'Hips')
        self.spine = find_bone(arm, 'Spine')
        self.chest = find_bone(arm, 'Chest')
        self.uchest = find_bone(arm, 'UpperChest')
        self.neck = find_bone(arm, 'Neck')
        self.head = find_bone(arm, 'Head')
        self.b = {}
        for side, tag in (('L', '_L_'), ('R', '_R_')):
            self.b[side] = {
                'shoulder': find_bone(arm, tag + 'Shoulder'),
                'upperarm': find_bone(arm, tag + 'UpperArm'),
                'lowerarm': find_bone(arm, tag + 'LowerArm'),
                'hand': find_bone(arm, tag + 'Hand'),
                'upperleg': find_bone(arm, tag + 'UpperLeg'),
                'lowerleg': find_bone(arm, tag + 'LowerLeg'),
                'foot': find_bone(arm, tag + 'Foot'),
                'toe': find_bone(arm, tag + 'ToeBase'),
            }
        # kości włosów i biustu — dla wtórnego ruchu z opóźnioną fazą
        self.hair = [pb for pb in arm.pose.bones if 'Hair' in pb.name]
        self.bust = [pb for pb in arm.pose.bones if 'Bust' in pb.name]
        missing = [k for k, v in self.b['L'].items() if v is None]
        if self.hips is None or self.head is None or missing:
            raise RuntimeError(f'brakujace kosci: {missing}')

    def reset(self):
        """Wrót do pozy spoczynkowej (bind pose) — wszystko w T.

        Ustawiamy `matrix_basis` na tożsamość zamiast wołać
        `bpy.ops.pose.transforms_clear()`: operator potrzebuje kontekstu
        okna 3D View, którego w trybie `-b` nie ma, a przy okazji
        `select_all` na 73 kościach kosztowałoby więcej niż cały klip.
        """
        for pb in self.arm.pose.bones:
            pb.matrix_basis = Matrix.Identity(4)
        sync()

    def animated_bones(self):
        """Kości faktycznie animowane, w kolejności alfabetycznej — musi
        być powtarzalna między eksportami."""
        names = [self.hips.name, self.spine.name, self.chest.name,
                 self.uchest.name, self.neck.name, self.head.name]
        for side in ('L', 'R'):
            for k in ('shoulder', 'upperarm', 'lowerarm', 'hand',
                      'upperleg', 'lowerleg', 'foot', 'toe'):
                if self.b[side][k] is not None:
                    names.append(self.b[side][k].name)
        names += [pb.name for pb in self.hair]
        names += [pb.name for pb in self.bust]
        seen, out = set(), []
        for n in sorted(names):
            if n not in seen and n in self.arm.pose.bones:
                seen.add(n)
                out.append(n)
        return out


def hang_arms(rig, splay=0.10):
    """Opuszcza ręce z pozy T do pozy stojącej.

    W pozy T ramię wskazuje poziomo (oś X), a ma wskazywać w dół.
    Lewa ramię wskazuje -X, prawa +X, więc kierunki różnią się znakiem.
    """
    for side, sgn in (('L', -1.0), ('R', 1.0)):
        aim_dir(rig.b[side]['upperarm'], (sgn * splay, 0.0, -1.0))
        sync()


def pose_idle(rig, t, dur):
    """Oddech i przenoszenie ciężaru w miejscu. Fala 1 na cykl = 1 oddech."""
    w = TAU * t / dur
    rig.reset()
    hang_arms(rig)

    # biodra: pionowy oddech (ciało w górę przy wdechu) + przet w bok
    trans_world(rig.hips, (0.004 * math.sin(w), 0.0, 0.010 * math.sin(w)))
    sync()
    spin(rig.hips, 0.026 * math.sin(w), 'Z')
    spin(rig.hips, 0.014 * math.sin(w), 'X')
    sync()

    # kręgosłup: przeciwny zwrot, żeby głowa stała w miejscu
    rot_world(rig.spine, (0, 1, 0), 0.020 * math.sin(w + 0.5)); sync()
    rot_world(rig.chest, (0, 1, 0), 0.026 * math.sin(w + 0.9)); sync()
    rot_world(rig.uchest, (0, 1, 0), 0.018 * math.sin(w + 1.2)); sync()
    rot_world(rig.neck, (0, 1, 0), -0.030 * math.sin(w + 1.5)); sync()
    rot_world(rig.head, (0, 1, 0), -0.022 * math.sin(w + 1.8)); sync()
    rot_world(rig.head, (1, 0, 0), 0.012 * math.sin(2 * w)); sync()

    for side, ph in (('L', 0.0), ('R', math.pi)):
        s = math.sin(w + ph)
        rot_world(rig.b[side]['upperarm'], (1, 0, 0), 0.055 * s); sync()
        rot_world(rig.b[side]['lowerarm'], (1, 0, 0), 0.10 + 0.03 * s); sync()
        rot_world(rig.b[side]['hand'], (0, 1, 0), 0.05 * s); sync()

    # włosy i biust reagują z opóźnieniem — wtórny ruch, nie główna animacja
    for i, pb in enumerate(rig.hair):
        rot_world(pb, (1, 0, 0), 0.055 * math.sin(w - 0.9 - 0.11 * i)); sync()
    for i, pb in enumerate(rig.bust):
        rot_world(pb, (1, 0, 0), 0.020 * math.sin(w - 0.6 - 0.15 * i)); sync()


def pose_locomotion(rig, t, dur, run=False):
    """Cykl chodu albo biegu. Ta sama geometria, różna amplituda i tempo.

    Chód: kroki w 1.0 s, noga w przód ±24°, kolano do 55°.
    Bieg: kroki w 0.70 s, noga ±41°, kolano do 100°, mocniejszy pochył.
    """
    w = TAU * t / dur
    rig.reset()
    hang_arms(rig)

    if run:
        sw_leg, sw_arm = 0.72, 0.55
        knee, bob, lean = 1.75, 0.075, 0.21
    else:
        sw_leg, sw_arm = 0.42, 0.30
        knee, bob, lean = 0.95, 0.032, 0.09

    # pochył tułowia do przodu
    rot_world(rig.spine, (1, 0, 0), lean * 0.35); sync()
    rot_world(rig.chest, (1, 0, 0), lean * 0.35); sync()
    rot_world(rig.uchest, (1, 0, 0), lean * 0.30); sync()

    # biodra: podskakiwanie 2× na cykl (tępy i ostry rytm chodu) + przet
    trans_world(rig.hips, (0.0, 0.0, -bob * (1.0 - math.cos(2 * w)) * 0.5))
    sync()
    spin(rig.hips, 0.075 * math.sin(w), 'Z')
    spin(rig.hips, 0.030 * math.sin(2 * w), 'Y')
    spin(rig.hips, 0.045 * math.sin(w), 'X')
    sync()

    rot_world(rig.spine, (0, 1, 0), -0.055 * math.sin(w)); sync()
    rot_world(rig.chest, (0, 1, 0), -0.070 * math.sin(w)); sync()
    rot_world(rig.uchest, (0, 1, 0), -0.045 * math.sin(w)); sync()

    # głowa się nie pochyla wraz z tułowiem — inaczej postać „tapia się"
    rot_world(rig.neck, (1, 0, 0), -lean * 0.55); sync()
    rot_world(rig.head, (1, 0, 0), -lean * 0.45); sync()
    rot_world(rig.head, (0, 1, 0), -0.035 * math.sin(w)); sync()

    for side, ph in (('L', 0.0), ('R', math.pi)):
        p = w + ph
        s, c = math.sin(p), math.cos(p)
        leg = rig.b[side]
        rot_world(leg['upperleg'], (1, 0, 0), sw_leg * s); sync()
        # kolano zgina się, gdy noga idzie do przodu (faza przenoszenia)
        k = knee * max(0.0, math.sin(p - 0.9))
        if run:
            k += knee * 0.55 * max(0.0, -c)
        rot_world(leg['lowerleg'], (1, 0, 0), -k); sync()
        # stopa: grzbiet przy przenoszeniu, prostowanie przy odbiciu
        rot_world(leg['foot'], (1, 0, 0), 0.45 * math.sin(p + 2.2) - 0.10); sync()
        rot_world(leg['toe'], (1, 0, 0), 0.25 * max(0.0, math.sin(p + 1.0))); sync()

        # ręce w przeciwnym rytmie nóg
        rot_world(rig.b[side]['upperarm'], (1, 0, 0), -sw_arm * s); sync()
        bend = 0.55 if run else 0.28
        rot_world(rig.b[side]['lowerarm'], (1, 0, 0), bend + 0.18 * s); sync()
        rot_world(rig.b[side]['hand'], (1, 0, 0), -0.12 * s); sync()

    # włosy: przy biegu mocniejsze opóźnienie (2× na cykl kroków)
    drag = 0.10 if run else 0.05
    for i, pb in enumerate(rig.hair):
        rot_world(pb, (1, 0, 0), drag * math.sin(2 * w - 1.1 - 0.12 * i)); sync()
    for i, pb in enumerate(rig.bust):
        rot_world(pb, (1, 0, 0), drag * 0.5 * math.sin(2 * w - 0.8 - 0.15 * i)); sync()


def _differs(track):
    """Czy kość cokolwiek robi w tym klipie.

    Idle nie rusza palcami, a zapisanie 60 identycznych klatek dla 15 kości
    palców to tylko śmieć w pliku. Wystarczy porównać wszystkie klatki
    z pierwszą; koszt jest nieistotny przy 30 klatkach.
    """
    t0, r0 = track['T'][0][1], track['R'][0][1]
    for k in range(1, len(track['T'])):
        if any(abs(a - b) > 1e-6 for a, b in zip(track['T'][k][1], t0)):
            return True
        if any(abs(a - b) > 1e-6 for a, b in zip(track['R'][k][1], r0)):
            return True
    return False


def sample_clip(rig, fn, dur, bone_names):
    """Próbuje klip, zwraca {indeks: {'T':[(t,v3)], 'R':[(t,v4)], 'S':…}}.

    Zapisujemy `rest^-1 @ posed`, czyli T/R/S względem pozy spoczynkowej,
    a nie globalne. Dopiero w runtime złożymy to z macierzą świata, więc
    format pozostaje niezależny od położenia modelu na scenie.
    """
    n = int(round(dur * FPS))
    tracks = {i: {'T': [], 'R': [], 'S': []} for i in range(len(bone_names))}
    for f in range(n):
        t = f / FPS
        fn(rig, t, dur)
        sync()
        for i, name in enumerate(bone_names):
            pb = rig.arm.pose.bones.get(name)
            if pb is None:
                continue
            rel = pb.bone.matrix_local.inverted_safe() @ pb.matrix
            loc, quat, scl = rel.decompose()
            tracks[i]['T'].append((t, (loc.x, loc.y, loc.z)))
            tracks[i]['R'].append((t, (quat.x, quat.y, quat.z, quat.w)))
            tracks[i]['S'].append((t, (scl.x, scl.y, scl.z)))
    return {i: e for i, e in tracks.items() if _differs(e)}


def write_animm(path, bone_names, clips):
    """Zapisuje plik `.urananm` — patrz docstring modułu."""
    out = bytearray()
    out += b'URANANM1'
    out += struct.pack('<I', len(bone_names))
    for n in bone_names:
        b = n.encode('utf-8')
        out += struct.pack('<H', len(b)) + b
    out += struct.pack('<I', len(clips))
    for cname, dur, tracks in clips:
        cb = cname.encode('utf-8')
        out += struct.pack('<H', len(cb)) + cb
        out += struct.pack('<ffI', dur, FPS, len(tracks))
        for i in sorted(tracks):
            e = tracks[i]
            out += struct.pack('<I', i)
            for key in ('T', 'R', 'S'):
                ks = e[key]
                out += struct.pack('<I', len(ks))
                for tt, vv in ks:
                    out += struct.pack('<f', tt)
                    out += struct.pack(f'<{len(vv)}f', *vv)
    path.write_bytes(bytes(out))
    return len(out)


def main():
    argv = sys.argv[sys.argv.index('--') + 1:]
    src, dst = Path(argv[0]), Path(argv[1])

    gl = read_gltf_skeleton(src)
    joint_names = [gl['nodes'][j].get('name', '') for j in gl['skins'][0]['joints']]
    print(f'[urananm] kości w skin[0]: {len(joint_names)}', flush=True)

    arm = import_glb(src)
    rig = Rig(arm)
    print(f'[urananm] kości w Blenderze: {len(arm.pose.bones)}')
    print(f'[urananm] włosy: {len(rig.hair)}, biust: {len(rig.bust)}', flush=True)

    # Nazwy z Blendera muszą trafić w `skin.joints` — inaczej indeksy
    # z eksportu nie zgadzałyby się z macierzami w pliku GLB.
    jset = set(joint_names)
    for n in rig.animated_bones():
        if n not in jset:
            print(f'[urananm] UWAGA: kość {n!r} nie ma w skin.joints')
    bone_names = [n for n in rig.animated_bones() if n in jset]
    print(f'[urananm] animowanych kości: {len(bone_names)}', flush=True)

    spec = [
        ('Idle', 2.0, lambda r, t, d: pose_idle(r, t, d)),
        ('Walk', 1.0, lambda r, t, d: pose_locomotion(r, t, d, run=False)),
        ('Run', 0.70, lambda r, t, d: pose_locomotion(r, t, d, run=True)),
    ]

    clips = []
    for cname, dur, fn in spec:
        print(f'[urananm] próbkuję {cname} ({dur}s @ {FPS} fps)...', flush=True)
        raw = sample_clip(rig, fn, dur, bone_names)
        tracks = {joint_names.index(bone_names[i]): e for i, e in raw.items()}
        clips.append((cname, dur, tracks))
        print(f'[urananm]   {len(tracks)} kości w ruchu', flush=True)

    size = write_animm(dst, joint_names, clips)
    print(f'[urananm] zapisano {dst} ({size} B)')


if __name__ == '__main__':
    main()
