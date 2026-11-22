//! Le serveur dédié, sans fenêtre : `cargo run -p voxel_sim --bin voxel-server`. Il lit `server.ron`
//! (voir `voxel_sim::dedicated`) et ne lie aucune bibliothèque d'affichage. `--data-dir <dossier>` :
//! ses mondes, ses mods et `server.ron` sont là (voir `voxel_sim::paths`).

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,voxel_sim=info"),
    )
    .init();
    voxel_sim::dedicated::run()
}
