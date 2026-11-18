//! Le budget des transferts vers le GPU : un seul pour tout ce que le jeu envoie dans une image
//! (maillages des chunks, tuiles du terrain lointain, libérations).
//!
//! Compter des maillages ne dit rien de leur coût : un chunk de plaine pèse quelques kilo-octets,
//! un chunk de forêt dense plusieurs centaines. Le budget compte donc des **octets** (ce que le
//! pilote doit copier) et du **temps** (ce qu'un envoi a réellement coûté sur le thread principal),
//! et s'arrête au premier des deux.
//!
//! # Une grosse opération n'est jamais bloquée
//!
//! Le premier envoi de l'image est **toujours** accepté, quelle que soit sa taille : un plafond
//! d'octets qui refuserait un maillage plus gros que lui l'empêcherait de partir à jamais. Ce
//! dépassement est compté ([`TransferStats::over_budget`], [`TransferStats::worst_item_bytes`]) pour
//! qu'on voie ce que les opérations indivisibles coûtent vraiment.

use std::time::Duration;

/// Le budget d'une image. Voir la documentation du module.
#[derive(Debug, Clone, Copy)]
pub struct TransferBudget {
    max_bytes: usize,
    max_time: Duration,
    max_unloads: usize,
    bytes: usize,
    time: Duration,
    uploads: usize,
    unloads: usize,
}

impl TransferBudget {
    pub fn new(max_bytes: usize, max_time: Duration, max_unloads: usize) -> Self {
        Self {
            max_bytes,
            max_time,
            max_unloads,
            bytes: 0,
            time: Duration::ZERO,
            uploads: 0,
            unloads: 0,
        }
    }

    /// Un budget sans limite (les tests, les outils sans GPU).
    pub fn unlimited() -> Self {
        Self::new(usize::MAX, Duration::MAX, usize::MAX)
    }

    /// Un envoi de `bytes` octets tient-il encore ? Toujours oui pour le premier de l'image.
    pub fn allows_upload(&self, bytes: usize) -> bool {
        self.uploads == 0
            || (self.bytes.saturating_add(bytes) <= self.max_bytes && self.time < self.max_time)
    }

    /// Une libération tient-elle encore ? Elle ne prend pas d'octets mais du temps et un nombre.
    pub fn allows_unload(&self) -> bool {
        self.unloads < self.max_unloads && self.time < self.max_time
    }

    /// Note un envoi fait : sa taille et ce qu'il a coûté.
    pub fn record_upload(&mut self, bytes: usize, elapsed: Duration) {
        self.bytes = self.bytes.saturating_add(bytes);
        self.time = self.time.saturating_add(elapsed);
        self.uploads += 1;
    }

    /// Note une libération faite.
    pub fn record_unload(&mut self, elapsed: Duration) {
        self.time = self.time.saturating_add(elapsed);
        self.unloads += 1;
    }

    pub fn uploads(&self) -> usize {
        self.uploads
    }

    pub fn unloads(&self) -> usize {
        self.unloads
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn time(&self) -> Duration {
        self.time
    }

    /// Le budget a-t-il été dépassé (par un envoi indivisible plus gros que lui) ?
    pub fn exceeded(&self) -> bool {
        self.bytes > self.max_bytes || self.time > self.max_time
    }
}

/// Ce que les budgets d'une série d'images ont coûté : pour les logs et les mesures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransferStats {
    pub frames: u64,
    pub uploads: u64,
    pub bytes: u64,
    /// Images dont l'envoi a dépassé le budget (une opération indivisible plus grosse que lui).
    pub over_budget: u64,
    /// Le plus gros envoi, en octets.
    pub worst_item_bytes: usize,
    /// Le temps d'envoi d'image le plus long.
    pub worst_frame: Duration,
    /// Octets envoyés dans l'image la plus chargée.
    pub worst_frame_bytes: usize,
}

impl TransferStats {
    /// Ajoute le bilan d'une image : à appeler une fois par image, avec le budget qu'elle a consommé.
    pub fn record_frame(&mut self, budget: &TransferBudget, largest_item: usize) {
        self.frames += 1;
        self.uploads += budget.uploads() as u64;
        self.bytes += budget.bytes() as u64;
        if budget.exceeded() {
            self.over_budget += 1;
        }
        self.worst_item_bytes = self.worst_item_bytes.max(largest_item);
        self.worst_frame = self.worst_frame.max(budget.time());
        self.worst_frame_bytes = self.worst_frame_bytes.max(budget.bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn the_first_upload_always_goes_even_when_larger_than_the_budget() {
        let budget = TransferBudget::new(1000, 2 * MS, 8);
        assert!(budget.allows_upload(10_000_000));
    }

    #[test]
    fn bytes_stop_the_next_upload_but_never_the_first() {
        let mut budget = TransferBudget::new(1000, 100 * MS, 8);
        assert!(budget.allows_upload(600));
        budget.record_upload(600, MS);
        assert!(budget.allows_upload(400), "600 + 400 tient dans 1000");
        assert!(!budget.allows_upload(401), "600 + 401 dépasse 1000");
        // Une grosse opération indivisible passe en tête de l'image, jamais en deuxième.
        let mut big = TransferBudget::new(1000, 100 * MS, 8);
        assert!(big.allows_upload(50_000));
        big.record_upload(50_000, 3 * MS);
        assert!(big.exceeded());
        assert!(!big.allows_upload(1));
    }

    #[test]
    fn time_stops_the_next_upload_once_spent() {
        let mut budget = TransferBudget::new(usize::MAX, 2 * MS, 8);
        budget.record_upload(10, MS);
        assert!(budget.allows_upload(10));
        budget.record_upload(10, 2 * MS);
        assert!(!budget.allows_upload(10));
        assert!(!budget.allows_unload(), "le temps est dépensé");
    }

    #[test]
    fn unloads_are_counted_and_limited() {
        let mut budget = TransferBudget::new(usize::MAX, Duration::MAX, 3);
        for _ in 0..3 {
            assert!(budget.allows_unload());
            budget.record_unload(Duration::ZERO);
        }
        assert!(!budget.allows_unload());
        assert_eq!(budget.unloads(), 3);
    }

    #[test]
    fn unlimited_never_refuses() {
        let mut budget = TransferBudget::unlimited();
        for _ in 0..1000 {
            assert!(budget.allows_upload(usize::MAX / 2000));
            budget.record_upload(usize::MAX / 2000, MS);
            assert!(budget.allows_unload());
            budget.record_unload(MS);
        }
        assert!(!budget.exceeded());
    }

    #[test]
    fn stats_count_the_frames_over_budget_and_the_worst_cases() {
        let mut stats = TransferStats::default();
        let mut quiet = TransferBudget::new(1000, 5 * MS, 8);
        quiet.record_upload(500, MS);
        stats.record_frame(&quiet, 500);
        let mut heavy = TransferBudget::new(1000, 5 * MS, 8);
        heavy.record_upload(9000, 4 * MS);
        stats.record_frame(&heavy, 9000);
        assert_eq!(stats.frames, 2);
        assert_eq!(stats.uploads, 2);
        assert_eq!(stats.bytes, 9500);
        assert_eq!(stats.over_budget, 1);
        assert_eq!(stats.worst_item_bytes, 9000);
        assert_eq!(stats.worst_frame, 4 * MS);
        assert_eq!(stats.worst_frame_bytes, 9000);
    }
}
