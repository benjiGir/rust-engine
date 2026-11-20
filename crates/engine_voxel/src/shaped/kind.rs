//! Les sortes de pièces qu'on tient en main : une sorte d'objet par forme (couche, escalier, muret...).
//!
//! Le matériau n'est pas dans la sorte : il est rangé dans la pile (voir le jeu, `shape_materials.rs`).
//! La sorte dit seulement **quelle forme** l'objet pose, **combien d'objets** un bloc plein donne à
//! l'établi (le rendement) et dans quelle famille l'établi la range.

/// Les familles de l'établi de taille : la grille des formes est rangée par elles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Couches, dalles, microblocs, grains.
    Layers,
    /// Pentes, droites et en coin.
    Slopes,
    /// Escaliers, marches, poutres.
    Blocks,
    /// Tranches, demi-couches, panneaux, planchettes.
    Panels,
    /// Formes qui se raccordent à leurs voisines.
    Connected,
}

impl Family {
    pub const ALL: [Family; 5] = [
        Family::Layers,
        Family::Slopes,
        Family::Blocks,
        Family::Panels,
        Family::Connected,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Family::Layers => "Couches et dalles",
            Family::Slopes => "Pentes",
            Family::Blocks => "Escaliers et blocs",
            Family::Panels => "Tranches et panneaux",
            Family::Connected => "Formes raccordées",
        }
    }
}

/// Une sorte de pièce. Les numéros sont écrits dans les sauvegardes (voir `format.rs`) : ne jamais les
/// changer, seulement en ajouter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PieceKind {
    /// Une couche de 1/16.
    Layer = 0,
    /// Une dalle (8/16).
    Slab = 1,
    /// Un niveau de pente.
    SlopeLayer = 2,
    /// La pente pleine (niveau 16).
    Slope = 3,
    /// Un cube de 1/8 (2 × 2 × 2 cellules).
    Micro = 4,
    /// Un grain de 1/64.
    Nano = 5,
    /// Un escalier : une dalle et une demi-dalle.
    Stairs = 6,
    /// Un escalier vertical : trois quarts d'une colonne.
    VerticalStairs = 7,
    /// Une marche verticale : une colonne de 8 × 16 × 8 dans un coin.
    Step = 8,
    /// Une poutre : une colonne de 8 × 16 × 8 centrée.
    Beam = 9,
    /// Une tranche d'arête, redimensionnable (une barre de n × n le long d'une arête).
    Slice = 10,
    /// Une tranche de coin, redimensionnable (un cube de n × n × n dans un coin).
    CornerSlice = 11,
    /// Une demi-couche : la moitié d'une couche (16 × e × 8).
    HalfLayer = 12,
    /// Un demi-panneau : 16 × 8 × 2.
    HalfPanel = 13,
    /// Un panneau d'un quart : 8 × 8 × 2 dans un coin d'une face.
    QuarterPanel = 14,
    /// Une planchette : 16 × 16 × 1 collée à une face de la case.
    Board = 15,
    /// Un niveau de pente en coin sortant.
    CornerSlopeLayer = 16,
    /// Un niveau de pente en coin rentrant.
    InnerSlopeLayer = 17,
    /// Un muret.
    Wall = 18,
    /// Une barrière.
    Fence = 19,
    /// Un portillon.
    Gate = 20,
    /// Une vitre.
    Pane = 21,
    /// Une vitre à plat.
    FlatPane = 22,
}

impl PieceKind {
    pub const ALL: [PieceKind; 23] = [
        PieceKind::Layer,
        PieceKind::Slab,
        PieceKind::SlopeLayer,
        PieceKind::Slope,
        PieceKind::Micro,
        PieceKind::Nano,
        PieceKind::Stairs,
        PieceKind::VerticalStairs,
        PieceKind::Step,
        PieceKind::Beam,
        PieceKind::Slice,
        PieceKind::CornerSlice,
        PieceKind::HalfLayer,
        PieceKind::HalfPanel,
        PieceKind::QuarterPanel,
        PieceKind::Board,
        PieceKind::CornerSlopeLayer,
        PieceKind::InnerSlopeLayer,
        PieceKind::Wall,
        PieceKind::Fence,
        PieceKind::Gate,
        PieceKind::Pane,
        PieceKind::FlatPane,
    ];

    /// Le numéro écrit dans les sauvegardes.
    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(code: u8) -> Option<PieceKind> {
        Self::ALL.get(usize::from(code)).copied()
    }

    /// L'identifiant de l'objet.
    pub fn key(self) -> &'static str {
        match self {
            PieceKind::Layer => "core:shape_layer",
            PieceKind::Slab => "core:shape_slab",
            PieceKind::SlopeLayer => "core:shape_slope_layer",
            PieceKind::Slope => "core:shape_slope",
            PieceKind::Micro => "core:shape_micro",
            PieceKind::Nano => "core:shape_nano",
            PieceKind::Stairs => "core:shape_stairs",
            PieceKind::VerticalStairs => "core:shape_vertical_stairs",
            PieceKind::Step => "core:shape_step",
            PieceKind::Beam => "core:shape_beam",
            PieceKind::Slice => "core:shape_slice",
            PieceKind::CornerSlice => "core:shape_corner_slice",
            PieceKind::HalfLayer => "core:shape_half_layer",
            PieceKind::HalfPanel => "core:shape_half_panel",
            PieceKind::QuarterPanel => "core:shape_quarter_panel",
            PieceKind::Board => "core:shape_board",
            PieceKind::CornerSlopeLayer => "core:shape_corner_slope_layer",
            PieceKind::InnerSlopeLayer => "core:shape_inner_slope_layer",
            PieceKind::Wall => "core:shape_wall",
            PieceKind::Fence => "core:shape_fence",
            PieceKind::Gate => "core:shape_gate",
            PieceKind::Pane => "core:shape_pane",
            PieceKind::FlatPane => "core:shape_flat_pane",
        }
    }

    pub fn from_key(key: &str) -> Option<PieceKind> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    /// Le nom court de la forme (la clé sans `core:shape_`) : `slab`, `corner_slice`... Les commandes
    /// de la console s'en servent.
    pub fn short_name(self) -> &'static str {
        self.key().trim_start_matches("core:shape_")
    }

    pub fn from_short_name(name: &str) -> Option<PieceKind> {
        Self::ALL.into_iter().find(|k| k.short_name() == name)
    }

    /// Le nom de la forme (le matériau s'y ajoute : « Dalle de chêne »).
    pub fn name(self) -> &'static str {
        match self {
            PieceKind::Layer => "Couche",
            PieceKind::Slab => "Dalle",
            PieceKind::SlopeLayer => "Couche de pente",
            PieceKind::Slope => "Pente",
            PieceKind::Micro => "Microbloc",
            PieceKind::Nano => "Grain",
            PieceKind::Stairs => "Escalier",
            PieceKind::VerticalStairs => "Escalier vertical",
            PieceKind::Step => "Marche verticale",
            PieceKind::Beam => "Poutre",
            PieceKind::Slice => "Tranche",
            PieceKind::CornerSlice => "Tranche de coin",
            PieceKind::HalfLayer => "Demi-couche",
            PieceKind::HalfPanel => "Demi-panneau",
            PieceKind::QuarterPanel => "Panneau d'un quart",
            PieceKind::Board => "Planchette",
            PieceKind::CornerSlopeLayer => "Couche de pente en coin",
            PieceKind::InnerSlopeLayer => "Couche de pente rentrante",
            PieceKind::Wall => "Muret",
            PieceKind::Fence => "Barrière",
            PieceKind::Gate => "Portillon",
            PieceKind::Pane => "Vitre",
            PieceKind::FlatPane => "Vitre à plat",
        }
    }

    /// Combien de pièces donne un bloc plein, et combien il en faut pour le reconstituer.
    ///
    /// L'escalier, l'escalier vertical, le muret et le portillon valent un bloc : on y garde 3/4 de la
    /// matière (comme un escalier), le reste est perdu dans la taille. Une tranche vaut une barre de
    /// 1 × 1 × 16 (256 par bloc) ; une tranche de coin vaut 8 seizièmes cubes (512 par bloc).
    pub fn per_block(self) -> u32 {
        match self {
            PieceKind::Layer
            | PieceKind::SlopeLayer
            | PieceKind::HalfPanel
            | PieceKind::Board
            | PieceKind::CornerSlopeLayer
            | PieceKind::InnerSlopeLayer => 16,
            PieceKind::Slab | PieceKind::Slope | PieceKind::Fence => 2,
            PieceKind::Micro => 8,
            PieceKind::Nano => 64,
            PieceKind::Stairs | PieceKind::VerticalStairs | PieceKind::Wall | PieceKind::Gate => 1,
            PieceKind::Step | PieceKind::Beam => 4,
            PieceKind::Slice => 256,
            PieceKind::CornerSlice => 512,
            PieceKind::HalfLayer | PieceKind::QuarterPanel => 32,
            PieceKind::Pane | PieceKind::FlatPane => 8,
        }
    }

    pub fn is_slope(self) -> bool {
        matches!(
            self,
            PieceKind::SlopeLayer
                | PieceKind::Slope
                | PieceKind::CornerSlopeLayer
                | PieceKind::InnerSlopeLayer
        )
    }

    /// Une forme qui se raccorde à ses voisines (seule dans sa case).
    pub fn is_connected(self) -> bool {
        matches!(
            self,
            PieceKind::Wall
                | PieceKind::Fence
                | PieceKind::Gate
                | PieceKind::Pane
                | PieceKind::FlatPane
        )
    }

    /// La famille de l'établi de taille.
    pub fn family(self) -> Family {
        match self {
            PieceKind::Layer | PieceKind::Slab | PieceKind::Micro | PieceKind::Nano => {
                Family::Layers
            }
            PieceKind::SlopeLayer
            | PieceKind::Slope
            | PieceKind::CornerSlopeLayer
            | PieceKind::InnerSlopeLayer => Family::Slopes,
            PieceKind::Stairs | PieceKind::VerticalStairs | PieceKind::Step | PieceKind::Beam => {
                Family::Blocks
            }
            PieceKind::Slice
            | PieceKind::CornerSlice
            | PieceKind::HalfLayer
            | PieceKind::HalfPanel
            | PieceKind::QuarterPanel
            | PieceKind::Board => Family::Panels,
            PieceKind::Wall
            | PieceKind::Fence
            | PieceKind::Gate
            | PieceKind::Pane
            | PieceKind::FlatPane => Family::Connected,
        }
    }

    /// Cette sorte grossit-elle quand on en pose une autre dessus (tranches) ?
    pub fn is_resizable(self) -> bool {
        matches!(self, PieceKind::Slice | PieceKind::CornerSlice)
    }

    /// Les formes d'une famille, dans l'ordre de l'établi.
    pub fn of_family(family: Family) -> impl Iterator<Item = PieceKind> {
        Self::ALL.into_iter().filter(move |k| k.family() == family)
    }
}

/// Le nombre d'objets « tranche » que vaut une tranche d'arête de `n` × `n` : une barre de 1 × 1 × 16
/// par seizième carré. Grossir de n à n + 1 coûte la différence ; casser rend le total : rien ne se
/// perd ni ne se crée.
pub fn slice_cost(n: u32) -> u32 {
    n * n
}

/// Le nombre d'objets « tranche de coin » que vaut un cube de `n` × `n` × `n` : un objet vaut 8
/// seizièmes cubes, arrondi au-dessus (un cube de 1 vaut 1 objet).
pub fn corner_slice_cost(n: u32) -> u32 {
    (n * n * n).div_ceil(8)
}

/// Une pile de cette sorte d'objet s'empile-t-elle par son `damage` (le numéro de matériau) ?
/// Les autres objets réservent `damage` à l'usure : seules les piles neuves s'empilent.
pub fn stacks_by_damage(item_key: &str) -> bool {
    PieceKind::from_key(item_key).is_some()
}
