#!/usr/bin/env python3
"""Import explicite des prises amont (voir audio/CREDITS.md), puis génération Rust hors ligne.
Usage : python3 tools/assetgen/import_audio.py /chemin/du/cache
Les noms des archives et les conversions sont contrôlés, aucun téléchargement implicite.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[2]
DEST = ROOT / "tools/assetgen/audio"
CACHE = Path(sys.argv[1])
records = []
recipes = []

def convert(source, target, channels=1, filters=None, music=False, provenance=None):
    target = DEST / target
    target.parent.mkdir(parents=True, exist_ok=True)
    cmd = ["ffmpeg", "-nostdin", "-v", "error", "-y", "-i", str(source), "-map_metadata", "-1", "-ac", str(channels), "-ar", "44100"]
    if filters:
        cmd += ["-af", filters]
    cmd += (["-c:a", "vorbis", "-strict", "-2", "-q:a", "6", "-fflags", "+bitexact"] if music else ["-c:a", "pcm_s16le", "-fflags", "+bitexact"])
    subprocess.run(cmd + [str(target)], check=True)
    records.append({"file": str(target.relative_to(DEST)), "sha256": hashlib.sha256(target.read_bytes()).hexdigest(), **(provenance or {})})
    return str(target.relative_to(DEST))

def recipe(name, source, kind="Effect", lowpass=None):
    fields = [f'name: "{name}"', f'source: "{source}"', f'kind: {kind}']
    if lowpass:
        fields += [f'lowpass: Some({lowpass}.0)']
    recipes.append("    (" + ", ".join(fields) + "),")

impact = zipfile.ZipFile(CACHE / "impact-sounds.zip")
for archive, slug in [(impact, "impact-sounds")]:
    (DEST / "licenses").mkdir(exist_ok=True)
    (DEST / "licenses" / f"kenney-{slug}.txt").write_bytes(archive.read("License.txt"))

# Quatre prises réellement différentes, et non quatre transpositions du même choc.
families = {
    "step_soft": ("footstep_grass", 7500), "step_hard": ("footstep_concrete", 7500),
    "step_snow": ("footstep_snow", 7000), "step_wood": ("footstep_wood", 7500),
    "step_metal": ("impactMetal_light", 6500),
    "break_soft": ("impactSoft_heavy", 7000), "break_hard": ("impactMining", 7500),
    "break_glass": ("impactGlass_heavy", 6000), "break_wood": ("impactWood_heavy", 7500),
    "break_metal": ("impactMetal_heavy", 6500),
    "place_soft": ("impactSoft_medium", 7000), "place_hard": ("impactGeneric_light", 7500),
    "place_wood": ("impactWood_medium", 7500), "place_metal": ("impactMetal_medium", 6500),
}
for name, (stem, lowpass) in families.items():
    for i in range(4):
        original = f"Audio/{stem}_{i:03}.ogg"
        local = CACHE / (stem + f"_{i:03}.ogg")
        local.write_bytes(impact.read(original))
        source = f"kenney/{stem}_{i:03}.wav"
        convert(local, source, provenance={"author": "Kenney", "page": "https://kenney.nl/assets/impact-sounds", "license": "CC0-1.0", "original": original})
        recipe(name + (f"_{i}" if i else ""), source, lowpass=lowpass)

nature = CACHE / "nature/Nature Sounds Pack"
# Sol meuble : les prises sont conservées sous leur nom d'origine (herbe sèche/terre).
for i in range(4):
    original = f"Footstep/FootstepGrass{i+5:02}.wav"
    source = convert(nature / original, f"nature/FootstepGrass{i+5:02}.wav", filters="silenceremove=start_periods=1:start_duration=0.005:start_threshold=-40dB:start_silence=0.01,atrim=duration=0.38", provenance={"author": "Antoine Goumain (Antoinemax)", "page": "https://opengameart.org/content/nature-sounds-pack", "license": "CC-BY-4.0", "original": original, "conversion_filter": "silenceremove=start_periods=1:start_duration=0.005:start_threshold=-40dB:start_silence=0.01,atrim=duration=0.38"})
    recipe("step_sand" + (f"_{i}" if i else ""), source, lowpass=6500)

# Boucles et appels naturels. Chaque coupe, canal et transformation figure ici.
ambient = [
    ("ambience_day", nature / "Ambient/AmbientNatureOutside.wav", "nature/outside.wav", "Loop", "Antoine Goumain (Antoinemax)", "nature-sounds-pack", "CC-BY-4.0", 2, None, None),
    ("ambience_night", CACHE / "crickets.mp3", "field/crickets.wav", "Loop", "Ted Kerr (Wolfgang_)", "crickets-ambient-noise-loopable", "CC0-1.0", 2, None, None),
    ("birds", CACHE / "forest.mp3", "field/birds.wav", "Ambient", "TinyWorlds", "forest-ambience", "CC0-1.0", 2, "atrim=start=5:duration=7,asetpts=PTS-STARTPTS", None),
    ("owl", CACHE / "owl.mp3", "field/owl.wav", "Ambient", "Breviceps", "https://freesound.org/people/Breviceps/sounds/465697/", "CC0-1.0", 1, None, None),
    ("waves", CACHE / "wave.flac", "field/wave.wav", "Ambient", "jasinski (sélection qubodup)", "beach-ocean-waves", "CC0-1.0", 2, None, None),
    ("rain", CACHE / "water/loop_rain.ogg", "water/rain.wav", "Loop", "rubberduck", "40-cc0-water-splash-slime-sfx", "CC0-1.0", 2, None, None),
    ("thunder", nature / "Ambient/AmbientNatureStormRainThunder.wav", "nature/thunder.wav", "Ambient", "Antoine Goumain (Antoinemax)", "nature-sounds-pack", "CC-BY-4.0", 2, None, None),
    ("cave_rumble", nature / "Ambient/AmbientNatureStormRainThunder02.wav", "nature/rumble.wav", "Ambient", "Antoine Goumain (Antoinemax)", "nature-sounds-pack", "CC-BY-4.0", 2, "atrim=start=2:duration=5,asetpts=PTS-STARTPTS", 240),
    ("bubbles", CACHE / "bubble.wav", "water/bubbles.wav", "Effect", "BMacZero", "bubble-sound-effects", "CC0-1.0", 1, "atrim=duration=1.4", 7000),
]
for name, path, target, kind, author, page, license_id, channels, filters, lowpass in ambient:
    page = page if page.startswith("https:") else "https://opengameart.org/content/" + page
    source = convert(path, target, channels, filters, provenance={"author": author, "page": page, "license": license_id, "original": path.name, "conversion_filter": filters})
    recipe(name, source, kind, lowpass)
for i in range(4):
    stem = f"splash_{i+1:02}"
    source = convert(CACHE / f"water/{stem}.ogg", f"water/{stem}.wav", provenance={"author": "rubberduck", "page": "https://opengameart.org/content/40-cc0-water-splash-slime-sfx", "license": "CC0-1.0", "original": stem + ".ogg"})
    recipe("splash" + (f"_{i}" if i else ""), source, lowpass=8000)
for i in range(3):
    stem = f"swallow-{i+1:02}"
    source = convert(CACHE / f"bottle/qubodup-bottle-and-drink-sounds/{stem}.flac", f"water/{stem}.wav", provenance={"author": "qubodup", "page": "https://opengameart.org/content/liquid-bottle-drink-set", "license": "CC0-1.0", "original": stem + ".flac"})
    recipe("gulp" + (f"_{i}" if i else ""), source, lowpass=7000)
# Une petite bulle isolée pour les gouttes, avec une queue de réverbération à l'import.
source = convert(CACHE / "water/bubble_01.ogg", "water/drip.wav", filters="aecho=0.8:0.7:155|340:0.25|0.12", provenance={"author": "rubberduck", "page": "https://opengameart.org/content/40-cc0-water-splash-slime-sfx", "license": "CC0-1.0", "original": "bubble_01.ogg", "conversion_filter": "aecho=0.8:0.7:155|340:0.25|0.12"})
recipe("drip", source, "Ambient", 7500)
local = CACHE / "bell.ogg"
local.write_bytes(impact.read("Audio/impactBell_heavy_000.ogg"))
source = convert(local, "kenney/bell.wav", provenance={"author": "Kenney", "page": "https://kenney.nl/assets/impact-sounds", "license": "CC0-1.0", "original": "Audio/impactBell_heavy_000.ogg"})
recipe("enchant", source, "Chime", 5000)

music = [
    ("music_day1", "day1.mp3", "calm-piano-1-vaporware"),
    ("music_day2", "night.mp3", "calm-relax-1-synthwave-421k"),
    ("music_night", "day2.mp3", "calm-ambient-1-synthwave-4k"),
    ("music_cave", "cave.mp3", "sirens-in-darkness"),
]
original_names = {"day1.mp3": "003_Vaporware_2.mp3", "night.mp3": "007_Synthwave_421k.mp3", "day2.mp3": "001_Synthwave_4k_0.mp3", "cave.mp3": "012_Sirens_in_Darkness_0.mp3"}
for name, local, page in music:
    source = convert(CACHE / local, f"music/{name}.ogg", 2,
                     "loudnorm=I=-23:TP=-3:LRA=11,afade=t=in:d=1.5", music=True,
                     provenance={"author": "The Cynic Project / cynicmusic.com / pixelsphere.org", "page": "https://opengameart.org/content/" + page, "license": "CC0-1.0", "original": original_names[local], "conversion_filter": "loudnorm=I=-23:TP=-3:LRA=11,afade=t=in:d=1.5; Vorbis (ffmpeg natif) q6 44100 stereo"})
    recipe(name, source, "Music")
(DEST / "bank.ron").write_text("// Montage et niveaux dans src/sounds.rs ; provenance détaillée dans sources.json.\n[\n" + "\n".join(recipes) + "\n]\n")
# Conserver les prises déjà importées lorsqu'une seule conversion est reprise.
old = json.loads((DEST / "sources.json").read_text()) if (DEST / "sources.json").exists() else []
merged = {r["file"]: r for r in old}
merged.update({r["file"]: r for r in records})
(DEST / "sources.json").write_text(json.dumps(list(merged.values()), ensure_ascii=False, indent=2) + "\n")
print(f"{len(recipes)} sons, {len(merged)} sources créditées")
