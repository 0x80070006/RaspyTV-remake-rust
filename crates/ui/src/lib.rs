//! Accueil et paramètres de RaspyTV.
//!
//! - [`model`] et [`render`] sont du Rust pur, testé nativement ;
//! - `app` (WebAssembly seulement) branche ce rendu sur la page : événements,
//!   navigation spatiale, bandeau, mode Tableau, sons et dialogue avec le shell
//!   par `window.ipc.postMessage`. Hors de RaspyTV (pas de `window.ipc`), la page
//!   tourne avec les données fictives de [`mock`] : c'est la démonstration.

pub mod mock;
pub mod model;
pub mod render;

#[cfg(target_arch = "wasm32")]
mod app;
