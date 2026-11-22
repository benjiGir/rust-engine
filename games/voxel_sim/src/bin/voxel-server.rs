//! Le serveur dédié, sans fenêtre : `cargo run -p voxel_sim --bin voxel-server`. Il lit `server.ron`
//! (voir `voxel_sim::dedicated`) et ne lie aucune bibliothèque d'affichage. `--data-dir <dossier>` :
//! ses mondes, ses mods et `server.ron` sont là (voir `voxel_sim::paths`).
//! Le journal s'écrit aussi dans `logs/server-latest.log`.

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    voxel_sim::paths::init(voxel_sim::paths::data_dir_arg(&args));
    voxel_sim::logs::init(voxel_sim::logs::Program::Server);
    voxel_sim::dedicated::run()
}
