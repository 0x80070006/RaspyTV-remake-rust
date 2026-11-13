//! Scripts injectés dans les sites des services.
//!
//! Ils restent en JavaScript : ils s'exécutent dans des pages tierces (YouTube,
//! Jellyfin Web…) dont la politique de sécurité interdit le WebAssembly.
//! Les messages vers RaspyTV passent par `window.ipc.postMessage` (wry) et sont
//! filtrés par le shell : seuls quelques messages sans danger sont acceptés.

use serde_json::Value;

/// CSS ajouté à chaque service : pas de barres de défilement, curseur masquable.
pub fn adapter_css(service_id: &str) -> String {
    let dashy = if service_id == "homelab" {
        "*,*::before,*::after{backdrop-filter:none!important;transition-duration:.01ms!important;animation-duration:.01ms!important;animation-iteration-count:1!important}html{scroll-behavior:auto!important}video,[class*=\"animated-background\"],[class*=\"particle\"]{display:none!important}"
    } else {
        ""
    };
    format!("html,body{{overscroll-behavior:none!important}}html{{scrollbar-width:none!important}}*{{scrollbar-width:none!important}}::-webkit-scrollbar{{width:0!important;height:0!important;display:none!important}}html.raspy-cursor-hidden,html.raspy-cursor-hidden *{{cursor:none!important}}{dashy}")
}

/// Pont commun : Échap / Début, clavier virtuel, curseur masqué après 8 s.
const BRIDGE: &str = r#"
const post=m=>{try{window.ipc.postMessage(JSON.stringify(m))}catch(e){}};
const info=console.info.bind(console);
console.info=(...a)=>{if(a[0]==='__RASPY_OSK_OPEN__'||a[0]==='__RASPY_OSK_CLOSE__'){post({t:'osk',open:a[0]==='__RASPY_OSK_OPEN__'});return}info(...a)};
addEventListener('keydown',e=>{if(e.ctrlKey&&e.altKey&&(e.key==='h'||e.key==='H')){e.preventDefault();post({t:'key',key:'Home'});return}if(e.key==='Escape'||e.key==='Home'){if(e.key==='Escape'&&document.fullscreenElement)return;e.preventDefault();e.stopImmediatePropagation();post({t:'key',key:e.key})}},true);
const noHoverFocus=e=>{if(window.ApiClient||document.querySelector('.skinHeader'))e.stopImmediatePropagation()};
addEventListener('pointerenter',noHoverFocus,true);addEventListener('mouseenter',noHoverFocus,true);
let timer,focusTimer;const root=document.documentElement,hide=()=>root.classList.add('raspy-cursor-hidden'),show=()=>{root.classList.remove('raspy-cursor-hidden');clearTimeout(timer);timer=setTimeout(hide,8000)};
const editable=el=>!!el?.closest?.('input:not([type=button]):not([type=submit]):not([type=checkbox]):not([type=radio]):not([type=range]),textarea,[contenteditable=true],[role=textbox]');
addEventListener('pointermove',show,{passive:true});addEventListener('pointerdown',show,{passive:true});addEventListener('wheel',show,{passive:true});addEventListener('keydown',hide,{passive:true});addEventListener('touchstart',hide,{passive:true});
addEventListener('focusin',e=>{clearTimeout(focusTimer);if(editable(e.target)&&!window.__raspyNavFocusing)post({t:'osk',open:true})},true);
addEventListener('focusout',()=>{clearTimeout(focusTimer);focusTimer=setTimeout(()=>{if(!editable(document.activeElement))post({t:'osk',open:false})},180)},true);
hide();
"#;

/// Script d'initialisation d'un service : CSS, pont et adaptateur de navigation
/// (`tv-input-adapter.js`), exécutés une fois le document prêt.
pub fn service_init_script(service_id: &str, tv_adapter: &str) -> String {
    let css = Value::String(adapter_css(service_id));
    format!(
        "(()=>{{if(window.__raspyBridge)return;window.__raspyBridge=true;{BRIDGE}const run=()=>{{try{{const s=document.createElement('style');s.textContent={css};(document.head||document.documentElement).appendChild(s)}}catch(e){{}}try{{{tv_adapter}\n}}catch(e){{console.error('RaspyTV adapter',e)}}}};if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',run,{{once:true}});else run();}})();"
    )
}

/// Réglages de NouTube (publicités bloquées, Shorts masqués, SponsorBlock).
pub const NOUTUBE_SETTINGS: &str = r#"{"blockAds":true,"hideShorts":true,"sponsorBlock":true}"#;

/// Script NouTube exécuté au tout début du document, sur YouTube uniquement.
/// Il note aussi les commandes « titre précédent / suivant » déclarées par la page.
pub fn noutube_init_script(source: &str) -> String {
    format!(
        r#"(()=>{{const host=location.hostname;if(!(host==='youtube.com'||host.endsWith('.youtube.com')))return;const settings={NOUTUBE_SETTINGS};let active=false;
try{{const session=navigator.mediaSession;if(session&&!window.__raspyMediaHandlers){{const handlers=window.__raspyMediaHandlers={{}};const declare=session.setActionHandler.bind(session);session.setActionHandler=(action,handler)=>{{handlers[action]=handler;return declare(action,handler)}}}}}}catch(e){{}}
window.NouTubeInitialSettings=settings;try{{localStorage.setItem('nou:settings',JSON.stringify(settings))}}catch(e){{}}
try{{
{source}
const applyShorts=()=>{{if(!settings.hideShorts||document.getElementById('noutube-shorts'))return;window.NouTube?.hideShorts?.();const guide=document.createElement('style');guide.textContent='ytd-guide-entry-renderer:has(a[title="Shorts"]),ytd-mini-guide-entry-renderer:has(a[title="Shorts"]){{display:none!important}}';document.head.appendChild(guide)}};
if(document.head)applyShorts();else document.addEventListener('DOMContentLoaded',applyShorts,{{once:true}});
active=typeof window.NouTube==='object';
}}catch(error){{console.error('NouTube',error)}}
try{{window.ipc.postMessage(JSON.stringify({{t:'noutube',active,host}}))}}catch(e){{}}}})();"#
    )
}

/// Met en pause toutes les vidéos et musiques de la page (iframes comprises).
pub const PAUSE_SCRIPT: &str = "(()=>{const pauseDocument=doc=>{doc.querySelectorAll('video,audio').forEach(media=>{try{media.pause()}catch{}});doc.querySelectorAll('iframe').forEach(frame=>{try{if(frame.contentDocument)pauseDocument(frame.contentDocument)}catch{}})};pauseDocument(document);try{navigator.mediaSession.playbackState='paused'}catch{};return true})()";

/// Coupe / rétablit le son de la page (équivalent de `setAudioMuted`).
pub fn mute_script(muted: bool) -> String {
    format!("(()=>{{window.__raspyMuted={muted};document.querySelectorAll('video,audio').forEach(m=>{{try{{m.muted={muted}}}catch{{}}}});return true}})()")
}

/// Lecture en cours : titre, artiste, jaquette, position (pour le panneau PS).
pub const MEDIA_PROBE: &str = "JSON.stringify((()=>{const s=navigator.mediaSession,m=s&&s.metadata,els=[...document.querySelectorAll('video,audio')].filter(e=>e.duration>0||!e.paused),el=els.find(e=>!e.paused)||els.sort((a,b)=>(b.duration||0)-(a.duration||0))[0];if(!m&&!el)return null;const art=m&&m.artwork&&m.artwork.length?[...m.artwork].sort((a,b)=>parseInt(b.sizes||'0',10)-parseInt(a.sizes||'0',10))[0].src:'';return{title:m?String(m.title||''):'',artist:m?String(m.artist||''):'',artwork:String(art||''),position:el?el.currentTime:0,duration:el&&isFinite(el.duration)?el.duration:0,playing:!!el&&!el.paused}})())";

/// Commande du panneau : lecture/pause, suivant, précédent, avance à une position.
pub fn media_command(op: &str, value: f64) -> Option<String> {
    if !["toggle", "next", "previous", "seek"].contains(&op) {
        return None;
    }
    let value = if value.is_finite() { value.max(0.0) } else { 0.0 };
    Some(format!(
        "(()=>{{const op='{op}',value={value},els=[...document.querySelectorAll('video,audio')],el=els.find(e=>!e.paused)||els.find(e=>e.duration>0),click=list=>{{for(const q of list){{const b=document.querySelector(q);if(b){{b.click();return true}}}}return false}};if(op==='toggle'){{if(el){{el.paused?el.play().catch(()=>{{}}):el.pause();return true}}return click(['.play-pause-button','.ytp-play-button'])}}if(op==='next'||op==='previous'){{const name=op==='next'?'nexttrack':'previoustrack',handler=(window.__raspyMediaHandlers||{{}})[name];if(op==='previous'&&el&&el.currentTime>3)el.currentTime=0;if(typeof handler==='function'){{try{{handler({{action:name}});return 'handler'}}catch(e){{}}}}if(click(op==='next'?['ytmusic-player-bar .next-button','.ytp-next-button']:['ytmusic-player-bar .previous-button','.ytp-prev-button']))return 'button';if(op==='previous'&&el){{el.currentTime=0;return 'restart'}}return false}}if(op==='seek'&&el&&isFinite(el.duration)){{el.currentTime=Math.min(Math.max(0,el.duration-1),value);return true}}return false}})()"
    ))
}

/// Quitte le plein écran du site ; renvoie `true` s'il y en avait un.
pub const EXIT_FULLSCREEN: &str = "(()=>{if(!document.fullscreenElement)return false;document.exitFullscreen().catch(()=>{});return true})()";
pub const PLAY_PAUSE: &str = "window.__raspyTv?.playPause?.()===true";
pub const CONTEXT: &str = "window.__raspyTv?.context?.()";
pub const KEYBOARD_CLOSED: &str = "window.__raspyTv?.keyboardClosed?.()";
/// Retour arrière dans l'historique du site, s'il en a un au-delà de la page d'entrée.
pub const HISTORY_LENGTH: &str = "history.length";

/// Touche envoyée par la manette à une page (navigation de `tv-input-adapter.js`).
pub fn key_script(key: &str) -> Option<String> {
    let allowed = ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Enter", "Escape", "Backspace", "Tab"];
    allowed.contains(&key).then(|| {
        format!("(()=>{{const t=document.activeElement||document.body;const o={{key:'{key}',code:'{key}',bubbles:true,cancelable:true}};const go=t.dispatchEvent(new KeyboardEvent('keydown',o));t.dispatchEvent(new KeyboardEvent('keyup',o));if(go&&'{key}'==='Enter'&&t.click&&t!==document.body)t.click();return true}})()")
    })
}

/// Texte saisi au clavier virtuel quand l'injection native n'est pas disponible.
pub fn insert_text_script(text: &str) -> String {
    let text = Value::String(text.chars().take(8).collect());
    format!("(()=>{{const t=document.activeElement;if(!t)return false;if(document.execCommand('insertText',false,{text}))return true;if('value' in t){{const s=t.selectionStart??t.value.length,e=t.selectionEnd??s;t.value=t.value.slice(0,s)+{text}+t.value.slice(e);t.selectionStart=t.selectionEnd=s+{text}.length;t.dispatchEvent(new Event('input',{{bubbles:true}}));return true}}return false}})()")
}

/// Touche spéciale du clavier virtuel (effacer, valider, déplacer le curseur…).
pub fn special_key_script(key: &str) -> Option<String> {
    let body = match key {
        "backspace" => "document.execCommand('delete')",
        "delete" => "document.execCommand('forwardDelete')",
        "enter" => "(()=>{const o={key:'Enter',code:'Enter',keyCode:13,bubbles:true,cancelable:true};if(t.dispatchEvent(new KeyboardEvent('keydown',o))){t.dispatchEvent(new KeyboardEvent('keypress',o));const f=t.form;if(f&&t.tagName!=='TEXTAREA'){f.requestSubmit?f.requestSubmit():f.submit()}}t.dispatchEvent(new KeyboardEvent('keyup',o))})()",
        "tab" | "up" | "down" | "escape" => {
            let name = match key { "tab" => "Tab", "up" => "ArrowUp", "down" => "ArrowDown", _ => "Escape" };
            return Some(format!("(()=>{{const t=document.activeElement||document.body;const o={{key:'{name}',code:'{name}',bubbles:true,cancelable:true}};t.dispatchEvent(new KeyboardEvent('keydown',o));t.dispatchEvent(new KeyboardEvent('keyup',o));return true}})()"));
        }
        "left" => "(()=>{if('selectionStart' in t&&t.selectionStart!==null){const p=Math.max(0,t.selectionStart-1);t.setSelectionRange(p,p)}})()",
        "right" => "(()=>{if('selectionStart' in t&&t.selectionStart!==null){const p=Math.min(t.value.length,t.selectionEnd+1);t.setSelectionRange(p,p)}})()",
        _ => return None,
    };
    Some(format!("(()=>{{const t=document.activeElement;if(!t)return false;{body};return true}})()"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_keys() {
        assert!(special_key_script("backspace").unwrap().contains("delete"));
        assert!(special_key_script("enter").unwrap().contains("requestSubmit"));
        assert!(special_key_script("rm").is_none());
    }

    #[test]
    fn media_commands_are_whitelisted() {
        assert!(media_command("toggle", 0.0).is_some());
        assert!(media_command("seek", f64::NAN).unwrap().contains("value=0"));
        assert!(media_command("eval", 0.0).is_none());
        assert!(key_script("ArrowUp").is_some());
        assert!(key_script("');alert(1);//").is_none());
    }

    #[test]
    fn scripts_escape_text() {
        assert!(insert_text_script("a'\"b").contains(r#""a'\"b""#));
        assert!(service_init_script("homelab", "/*adapter*/").contains("backdrop-filter"));
        assert!(noutube_init_script("/*src*/").contains("/*src*/"));
    }
}
