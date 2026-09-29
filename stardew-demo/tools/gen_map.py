"""Generuje `stardew-demo/assets/maps/farm.xml` — farma z wybudowanym domkiem.

Mapa jest pisana w czytelnym XML-u (format `uran-tilemap`), żeby dało się ją
poprawiać ręcznie w edytorze tekstowym.
"""

W, H = 40, 30          # rozmiar mapy w kaflach
TILE = 16              # rozmiar kafla w świecie (px)

# --- Indeksy kafli -------------------------------------------------------
# Arkusz `terrain_spring_expanded` (16x16, 256x256 px).
# Kolumny 0-4 to podłoże (trawa/piasek), 8-9 kamień, 10+ woda.
# Kafle liczymy `y * 16 + x`, bo arkusz ma 16 kolumn.
def t(x, y):
    return y * 16 + x


GRASS = t(1, 1)         # jasna trawa (środek, nie krawędź)
GRASS_LIGHT = t(2, 1)   # jaśniejszy wariant trawy
GRASS_DARK = t(0, 1)    # ciemniejszy wariant trawy
SAND = t(0, 2)          # piaszczysta ścieżka
SAND_LIGHT = t(1, 2)
STONE = t(10, 2)        # kamień
WATER = t(10, 12)       # woda (środek)
WATER_EDGE = t(11, 12)

# --- Domek (arkusz `buildings`, 16 kolumn) --------------------------------
# Dom stoi w kolumnach 0-3, wierszach 0-5:
#   wiersze 0-3 = pomarańczowy dach,
#   wiersze 4-5 = drewniane ściany (kolumny 1-3 mają okna).
ROOF_R0 = [t(0, 0), t(1, 0), t(2, 0), t(3, 0)]
ROOF_R1 = [t(0, 1), t(1, 1), t(2, 1), t(3, 1)]
ROOF_R2 = [t(0, 2), t(1, 2), t(2, 2), t(3, 2)]
ROOF_R3 = [t(0, 3), t(1, 3), t(2, 3), t(3, 3)]
WALL_PLAIN = t(0, 4)
WALL_WIN = t(2, 4)
DOOR = t(0, 5)

# --- Dekoracje (arkusz `details_spring`, 16 kolumn) ----------------------
FENCE = t(1, 0)         # płotek (drewniany)
TREE = t(6, 0)          # drzewo z koroną
FLOWER = t(6, 3)        # kępa kwiatów
STONE_PILE = t(3, 1)    # kamyń


layers = {}


def layer(name):
    l = layers.setdefault(name, [[None] * W for _ in range(H)])
    return l


ground = layer("ground")
decor = layer("decor")
walls = layer("walls")

# --- Podłoga: trawa z delikatnym wzorem (dwie jaśniejsze plamy) -----------
# Wzór dajemy po to, żeby 40x30 kafli nie wyglądało jak jednolity kolor —
# to najtańszy sposób na czytelne „polanie" bez autotilingu.
for y in range(H):
    for x in range(W):
        if (x * 7 + y * 13) % 23 == 0:
            ground[y][x] = GRASS_LIGHT
        elif (x * 5 + y * 3) % 17 == 0:
            ground[y][x] = GRASS_DARK
        else:
            ground[y][x] = GRASS

# Ścieżka: od drzwi domu (16, 21) w dół do bramy (16, 1)
for y in range(1, 21):
    ground[y][16] = SAND
    ground[y][17] = SAND_LIGHT
# Odcinek do studni przy domu
for y in range(6, 11):
    ground[y][10] = SAND

# Staw w prawym górnym rogu (nieregularny kształt)
pond = [
    (28, 20), (29, 20), (30, 20),
    (28, 21), (29, 21), (30, 21), (31, 21),
    (28, 22), (29, 22), (30, 22), (31, 22),
    (29, 23), (30, 23),
]
for (x, y) in pond:
    ground[y][x] = WATER_EDGE if x in (28, 31) else WATER

# Kamienny placyk przed domem — od PROGU (wiersz 27) w gore do polaczki
# ze sciezka (wiersz 19).  Gracz startuje tutaj, wiec placyk musi siegac
# az za drzwi, inaczej spawn wypadalby na trawie.
for y in range(19, 28):
    for x in range(13, 18):
        ground[y][x] = STONE

# --- Domek: 4 kafle szeroko, 6 wierszy (dach 4 + ściany 2) --------------
# Budynek stoi na (13, 21)..(16, 26). Kolumny dachu odpowiadają 1:1
# kolumnom budynku, dzięki czemu dach jest symetryczny i nie „skacze".
HOUSE_X0, HOUSE_X1 = 13, 16
HOUSE_Y0, HOUSE_Y1 = 21, 26

ROOF_ROWS = [ROOF_R0, ROOF_R1, ROOF_R2, ROOF_R3]
for row_i, roof in enumerate(ROOF_ROWS):
    y = HOUSE_Y0 + row_i
    for col_i, x in enumerate(range(HOUSE_X0, HOUSE_X1 + 1)):
        walls[y][x] = roof[col_i]

# Ściany: dwa wiersze pod dachem, z oknem pośrodku i drzwiami na dole.
for x in range(HOUSE_X0, HOUSE_X1 + 1):
    walls[HOUSE_Y0 + 4][x] = WALL_WIN
    walls[HOUSE_Y0 + 5][x] = WALL_WIN
# Drzwi w środkowej kolumnie, najniższy wiersz.
walls[HOUSE_Y1][HOUSE_X0 + 2] = DOOR
# Narożniki ścian bez okna (mniej „okienkowato").
walls[HOUSE_Y0 + 4][HOUSE_X0] = WALL_PLAIN
walls[HOUSE_Y0 + 4][HOUSE_X1] = WALL_PLAIN

# --- Płot wokół pola uprawnego (po lewej) --------------------------------
FENCE_X0, FENCE_X1 = 3, 10
FENCE_Y0, FENCE_Y1 = 4, 12
for x in range(FENCE_X0, FENCE_X1 + 1):
    decor[FENCE_Y0][x] = FENCE
    decor[FENCE_Y1][x] = FENCE
for y in range(FENCE_Y0, FENCE_Y1 + 1):
    decor[y][FENCE_X0] = FENCE
    decor[y][FENCE_X1] = FENCE
# Wyrwa na bramie od strony ścieżki
decor[FENCE_Y0][7] = None
decor[FENCE_Y0][8] = None

# --- Sad (drzewa owocowe) przy stawie ------------------------------------
TREES = [(25, 16), (27, 16), (25, 18), (27, 18), (33, 16), (33, 18)]
for (x, y) in TREES:
    decor[y][x] = TREE

# Kwiaty wokół domu
FLOWER_SPOTS = [(12, 20), (17, 20), (12, 22), (17, 22), (12, 24), (17, 24)]
for (x, y) in FLOWER_SPOTS:
    decor[y][x] = FLOWER

# --- Kolizje: ściany domu + płot + drzewa + woda ------------------------
solid_props = []
# Dom: blokujemy cały obrys — wejście od strony ścieżki też (demo).
for y in range(HOUSE_Y0, HOUSE_Y1 + 1):
    for x in range(HOUSE_X0, HOUSE_X1 + 1):
        solid_props.append((x, y))
for y in range(FENCE_Y0, FENCE_Y1 + 1):
    for x in range(FENCE_X0, FENCE_X1 + 1):
        if decor[y][x] == FENCE:
            solid_props.append((x, y))
for (x, y) in TREES:
    solid_props.append((x, y))
for (x, y) in FLOWER_SPOTS:
    solid_props.append((x, y))
# Brzeg wody też jest nieprzechodni
for (x, y) in pond:
    solid_props.append((x, y))


def rows_xml(name, tileset, grid, props, extra=""):
    out = [f'  <layer name="{name}" tileset="{tileset}"{extra}>']
    for y, row in enumerate(grid):
        cells = " ".join("." if c is None else str(c) for c in row)
        out.append(f'    <row y="{y}">{cells}</row>')
    for (x, y) in props:
        out.append(f'    <props x="{x}" y="{y}">solid</props>')
    out.append("  </layer>")
    return "\n".join(out)


ASSET = "vectoraith_tileset_farming_sim_essentials/Original/16x16/Tilesets (Modular)"

xml = f'''<?xml version="1.0" encoding="UTF-8"?>
<!-- Farma demo — mapa budowana w XML.
     Edytuj `<row y="...">`, aby przesuwać kafle; indeksy liczone są
     od lewej góry arkusza. Linia `y="0"` to kafel na samym dole. -->
<map name="farma" tile-size="{TILE}" width="{W}" height="{H}">
  <!-- Punkt startowy gracza: na kamiennym placyku PRZED drzwiami domu.
      Wspolrzedne w pikselach kafla (16 px); demo skaluje je do swojego
      swiata. Kafel (15, 27) to tuz za progiem drzwi. -->
  <spawn x="248" y="440"/>

  <!-- Obszar, w którym gracz może przekopywać ziemię i zmieniać kafle.
      Wariant `tiles="true"` podaje liczby w kaflach, nie w pikselach. -->
  <edit-area x="3" y="4" width="8" height="9" tiles="true"/>

  <tileset name="terrain" src="{ASSET}/vectoraith_tileset_farmingsims_terrain_spring_expanded.png" tile-size="16"/>
  <tileset name="buildings" src="{ASSET}/vectoraith_tileset_farmingsims_buildings.png" tile-size="16"/>
  <tileset name="details" src="{ASSET}/vectoraith_tileset_farmingsims_details_spring.png" tile-size="16"/>

{rows_xml("ground", "terrain", ground, [])}
{rows_xml("decor", "details", decor, solid_props, ' solid="true"')}
{rows_xml("walls", "buildings", walls, [], ' solid="true"')}
</map>
'''

import os

# Skrypt leży w `stardew-demo/tools/`, a mapa musi trafić do `assets/maps/`.
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out = os.path.join(root, "assets", "maps", "farm.xml")
os.makedirs(os.path.dirname(out), exist_ok=True)
with open(out, "w", encoding="utf-8") as f:
    f.write(xml)
print("zapisano", out, len(xml), "bajtów")
