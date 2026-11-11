//! Translations of the messages the Rust code shows (the core's on-screen
//! messages, the desktop app's panel and help). Android, the TV app and the
//! phone remote page translate their own interface texts.
//!
//! Messages are looked up by their English text, gettext-style: `t("No
//! previous channel")`. Placeholders are `{0}`, `{1}`... ([`tf`]), so a
//! translation may put them in another order. Missing translations fall
//! back to English.

use std::sync::atomic::{AtomicU8, Ordering};

/// The languages IPTV speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En = 0,
    It = 1,
    Es = 2,
    Fr = 3,
    De = 4,
    Pt = 5,
}

static LANG: AtomicU8 = AtomicU8::new(0);

impl Lang {
    /// From a locale such as "it", "it-IT", "it_IT.UTF-8" or "pt-BR";
    /// English for anything else.
    pub fn from_code(code: &str) -> Lang {
        let code = code.trim().to_ascii_lowercase();
        match code.get(..2).unwrap_or("") {
            "it" => Lang::It,
            "es" => Lang::Es,
            "fr" => Lang::Fr,
            "de" => Lang::De,
            "pt" => Lang::Pt,
            _ => Lang::En,
        }
    }

    pub fn code(self) -> &'static str {
        ["en", "it", "es", "fr", "de", "pt"][self as usize]
    }
}

/// Sets the language for all messages (once, at startup).
pub fn set_language(code: &str) {
    LANG.store(Lang::from_code(code) as u8, Ordering::Relaxed);
}

pub fn language() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::It,
        2 => Lang::Es,
        3 => Lang::Fr,
        4 => Lang::De,
        5 => Lang::Pt,
        _ => Lang::En,
    }
}

/// The message in the current language.
pub fn t(english: &'static str) -> &'static str {
    t_in(language(), english)
}

/// The message in a given language.
pub fn t_in(lang: Lang, english: &'static str) -> &'static str {
    if lang == Lang::En {
        return english;
    }
    TABLE
        .iter()
        .find(|(en, _)| *en == english)
        .map(|(_, tr)| tr[lang as usize - 1])
        .unwrap_or(english)
}

/// The message with `{0}`, `{1}`... replaced by `args`.
pub fn tf(english: &'static str, args: &[&dyn std::fmt::Display]) -> String {
    let mut s = t(english).to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), &a.to_string());
    }
    s
}

/// The desktop app's key help, in the current language.
pub fn help() -> &'static str {
    HELP[language() as usize]
}

/// English -> [Italian, Spanish, French, German, Portuguese].
const TABLE: &[(&str, [&str; 5])] = &[
    // --- core: zapping and lists
    ("No working channels in this view", [
        "Nessun canale funzionante in questa vista",
        "No hay canales que funcionen en esta vista",
        "Aucune chaîne ne fonctionne dans cette vue",
        "Keine funktionierenden Sender in dieser Ansicht",
        "Nenhum canal a funcionar nesta vista",
    ]),
    ("No channels in this view", [
        "Nessun canale in questa vista",
        "No hay canales en esta vista",
        "Aucune chaîne dans cette vue",
        "Keine Sender in dieser Ansicht",
        "Nenhum canal nesta vista",
    ]),
    ("No previous channel", [
        "Nessun canale precedente",
        "No hay canal anterior",
        "Pas de chaîne précédente",
        "Kein vorheriger Sender",
        "Nenhum canal anterior",
    ]),
    ("No favorites yet — press 's' to add the current channel", [
        "Ancora nessun preferito — premi «s» per aggiungere il canale attuale",
        "Aún no hay favoritos — pulsa «s» para añadir el canal actual",
        "Pas encore de favoris — appuyez sur « s » pour ajouter la chaîne en cours",
        "Noch keine Favoriten — drücke „s“, um den aktuellen Sender hinzuzufügen",
        "Ainda não há favoritos — prima «s» para adicionar o canal atual",
    ]),
    ("No recently watched channels yet", [
        "Ancora nessun canale visto di recente",
        "Aún no hay canales vistos recientemente",
        "Aucune chaîne regardée récemment pour l'instant",
        "Noch keine kürzlich gesehenen Sender",
        "Ainda não há canais vistos recentemente",
    ]),
    ("Removed from favorites: {0}", [
        "Rimosso dai preferiti: {0}",
        "Quitado de favoritos: {0}",
        "Retiré des favoris : {0}",
        "Aus den Favoriten entfernt: {0}",
        "Removido dos favoritos: {0}",
    ]),
    ("★ Added to favorites: {0}", [
        "★ Aggiunto ai preferiti: {0}",
        "★ Añadido a favoritos: {0}",
        "★ Ajouté aux favoris : {0}",
        "★ Zu den Favoriten hinzugefügt: {0}",
        "★ Adicionado aos favoritos: {0}",
    ]),
    ("No new favorites: they were all here already", [
        "Nessun nuovo preferito: c'erano già tutti",
        "Ningún favorito nuevo: ya estaban todos",
        "Aucun nouveau favori : ils étaient déjà tous là",
        "Keine neuen Favoriten: Alle waren schon da",
        "Nenhum favorito novo: já estavam todos aqui",
    ]),
    ("★ Added 1 favorite", [
        "★ Aggiunto 1 preferito",
        "★ 1 favorito añadido",
        "★ 1 favori ajouté",
        "★ 1 Favorit hinzugefügt",
        "★ 1 favorito adicionado",
    ]),
    ("★ Added {0} favorites", [
        "★ Aggiunti {0} preferiti",
        "★ {0} favoritos añadidos",
        "★ {0} favoris ajoutés",
        "★ {0} Favoriten hinzugefügt",
        "★ {0} favoritos adicionados",
    ]),
    ("({0} not in this channel list; kept for when they are)", [
        "({0} non presenti in questo elenco; conservati per quando ci saranno)",
        "({0} no están en esta lista; se guardan para cuando estén)",
        "({0} absents de cette liste ; gardés pour quand ils y seront)",
        "({0} nicht in dieser Senderliste; bleiben gespeichert, bis sie da sind)",
        "({0} não estão nesta lista; guardados para quando estiverem)",
    ]),
    ("Showing: {0}", [
        "Mostrati: {0}",
        "Mostrando: {0}",
        "Affichage : {0}",
        "Angezeigt: {0}",
        "A mostrar: {0}",
    ]),
    ("All countries and languages", [
        "Tutti i paesi e le lingue",
        "Todos los países e idiomas",
        "Tous les pays et toutes les langues",
        "Alle Länder und Sprachen",
        "Todos os países e idiomas",
    ]),
    ("All channels", [
        "Tutti i canali",
        "Todos los canales",
        "Toutes les chaînes",
        "Alle Sender",
        "Todos os canais",
    ]),
    ("No channel {0}", [
        "Nessun canale {0}",
        "No existe el canal {0}",
        "Pas de chaîne {0}",
        "Kein Sender {0}",
        "Não existe o canal {0}",
    ]),
    ("Favorites", ["Preferiti", "Favoritos", "Favoris", "Favoriten", "Favoritos"]),
    ("Recently watched", [
        "Visti di recente",
        "Vistos recientemente",
        "Regardées récemment",
        "Zuletzt gesehen",
        "Vistos recentemente",
    ]),
    ("Search: {0}", ["Cerca: {0}", "Buscar: {0}", "Recherche : {0}", "Suche: {0}", "Pesquisa: {0}"]),
    ("Groups: {0}", ["Gruppi: {0}", "Grupos: {0}", "Groupes : {0}", "Gruppen: {0}", "Grupos: {0}"]),
    ("This isn't an IPTV favorites file.", [
        "Questo non è un file di preferiti di IPTV.",
        "Este no es un archivo de favoritos de IPTV.",
        "Ce n'est pas un fichier de favoris IPTV.",
        "Das ist keine IPTV-Favoritendatei.",
        "Este não é um ficheiro de favoritos do IPTV.",
    ]),
    ("This favorites file is from a newer IPTV: update this one first.", [
        "Questo file di preferiti viene da un IPTV più recente: aggiorna prima questo.",
        "Este archivo de favoritos es de un IPTV más reciente: actualiza este primero.",
        "Ce fichier de favoris vient d'un IPTV plus récent : mettez d'abord celui-ci à jour.",
        "Diese Favoritendatei stammt von einem neueren IPTV: Aktualisiere zuerst dieses.",
        "Este ficheiro de favoritos é de um IPTV mais recente: atualize primeiro este.",
    ]),
    // --- desktop: messages
    ("Loading channels…", [
        "Caricamento dei canali…",
        "Cargando canales…",
        "Chargement des chaînes…",
        "Sender werden geladen…",
        "A carregar os canais…",
    ]),
    ("Could not load the channel list. Check your internet connection.", [
        "Impossibile caricare l'elenco dei canali. Controlla la connessione a internet.",
        "No se pudo cargar la lista de canales. Comprueba tu conexión a internet.",
        "Impossible de charger la liste des chaînes. Vérifiez votre connexion internet.",
        "Die Senderliste konnte nicht geladen werden. Prüfe deine Internetverbindung.",
        "Não foi possível carregar a lista de canais. Verifique a ligação à internet.",
    ]),
    ("Updating IPTV to version {0}…\nIt will restart in a few seconds.", [
        "Aggiornamento di IPTV alla versione {0}…\nSi riavvierà tra pochi secondi.",
        "Actualizando IPTV a la versión {0}…\nSe reiniciará en unos segundos.",
        "Mise à jour d'IPTV vers la version {0}…\nIl redémarrera dans quelques secondes.",
        "IPTV wird auf Version {0} aktualisiert…\nEs startet in wenigen Sekunden neu.",
        "A atualizar o IPTV para a versão {0}…\nVai reiniciar dentro de alguns segundos.",
    ]),
    ("checking channels {0}%", [
        "verifica dei canali {0}%",
        "comprobando canales {0}%",
        "vérification des chaînes {0} %",
        "Sender werden geprüft: {0} %",
        "a verificar canais {0}%",
    ]),
    ("{0} audio languages  (a)", [
        "{0} lingue audio  (a)",
        "{0} idiomas de audio  (a)",
        "{0} langues audio  (a)",
        "{0} Audiosprachen  (a)",
        "{0} idiomas de áudio  (a)",
    ]),
    ("1 subtitle  (t)", [
        "1 sottotitolo  (t)",
        "1 subtítulo  (t)",
        "1 sous-titre  (t)",
        "1 Untertitel  (t)",
        "1 legenda  (t)",
    ]),
    ("{0} subtitles  (t)", [
        "{0} sottotitoli  (t)",
        "{0} subtítulos  (t)",
        "{0} sous-titres  (t)",
        "{0} Untertitel  (t)",
        "{0} legendas  (t)",
    ]),
    ("This channel has {0}", [
        "Questo canale ha {0}",
        "Este canal tiene {0}",
        "Cette chaîne propose {0}",
        "Dieser Sender hat {0}",
        "Este canal tem {0}",
    ]),
    ("This channel has a single audio language", [
        "Questo canale ha una sola lingua audio",
        "Este canal tiene un solo idioma de audio",
        "Cette chaîne n'a qu'une langue audio",
        "Dieser Sender hat nur eine Audiosprache",
        "Este canal tem um único idioma de áudio",
    ]),
    ("This channel has no subtitles", [
        "Questo canale non ha sottotitoli",
        "Este canal no tiene subtítulos",
        "Cette chaîne n'a pas de sous-titres",
        "Dieser Sender hat keine Untertitel",
        "Este canal não tem legendas",
    ]),
    ("Audio: {0}   ({1}/{2})", [
        "Audio: {0}   ({1}/{2})",
        "Audio: {0}   ({1}/{2})",
        "Audio : {0}   ({1}/{2})",
        "Audio: {0}   ({1}/{2})",
        "Áudio: {0}   ({1}/{2})",
    ]),
    ("Subtitles: {0}   ({1}/{2})", [
        "Sottotitoli: {0}   ({1}/{2})",
        "Subtítulos: {0}   ({1}/{2})",
        "Sous-titres : {0}   ({1}/{2})",
        "Untertitel: {0}   ({1}/{2})",
        "Legendas: {0}   ({1}/{2})",
    ]),
    ("Subtitles: off", [
        "Sottotitoli: disattivati",
        "Subtítulos: desactivados",
        "Sous-titres : désactivés",
        "Untertitel: aus",
        "Legendas: desligadas",
    ]),
    ("No other channel to show", [
        "Nessun altro canale da mostrare",
        "No hay otro canal que mostrar",
        "Aucune autre chaîne à afficher",
        "Kein anderer Sender zum Anzeigen",
        "Nenhum outro canal para mostrar",
    ]),
    ("{0} — small picture", [
        "{0} — riquadro",
        "{0} — imagen pequeña",
        "{0} — petite image",
        "{0} — kleines Bild",
        "{0} — imagem pequena",
    ]),
    ("Press p for a second channel in the corner", [
        "Premi p per un secondo canale nell'angolo",
        "Pulsa p para ver un segundo canal en la esquina",
        "Appuyez sur p pour une deuxième chaîne dans le coin",
        "Drücke p für einen zweiten Sender in der Ecke",
        "Prima p para um segundo canal no canto",
    ]),
    ("No favorites to export yet (s adds the current channel)", [
        "Ancora nessun preferito da esportare (s aggiunge il canale attuale)",
        "Aún no hay favoritos que exportar (s añade el canal actual)",
        "Pas encore de favoris à exporter (s ajoute la chaîne en cours)",
        "Noch keine Favoriten zum Exportieren (s fügt den aktuellen Sender hinzu)",
        "Ainda não há favoritos para exportar (s adiciona o canal atual)",
    ]),
    ("Favorites saved ({0}) to\n{1}\n(on another device: I, or the phone remote)", [
        "Preferiti salvati ({0}) in\n{1}\n(su un altro dispositivo: I, oppure il telecomando dal telefono)",
        "Favoritos guardados ({0}) en\n{1}\n(en otro dispositivo: I, o el mando desde el móvil)",
        "Favoris enregistrés ({0}) dans\n{1}\n(sur un autre appareil : I, ou la télécommande du téléphone)",
        "Favoriten gespeichert ({0}) in\n{1}\n(auf einem anderen Gerät: I oder die Handy-Fernbedienung)",
        "Favoritos guardados ({0}) em\n{1}\n(noutro dispositivo: I, ou o comando no telemóvel)",
    ]),
    ("Couldn't save the favorites to {0}: {1}", [
        "Impossibile salvare i preferiti in {0}: {1}",
        "No se pudieron guardar los favoritos en {0}: {1}",
        "Impossible d'enregistrer les favoris dans {0} : {1}",
        "Die Favoriten konnten nicht in {0} gespeichert werden: {1}",
        "Não foi possível guardar os favoritos em {0}: {1}",
    ]),
    ("Put “IPTV favorites.json” (from E on another device) here:\n{0}", [
        "Metti «IPTV favorites.json» (creato con E su un altro dispositivo) qui:\n{0}",
        "Pon «IPTV favorites.json» (creado con E en otro dispositivo) aquí:\n{0}",
        "Placez « IPTV favorites.json » (créé avec E sur un autre appareil) ici :\n{0}",
        "Lege „IPTV favorites.json“ (mit E auf einem anderen Gerät erstellt) hierhin:\n{0}",
        "Coloque «IPTV favorites.json» (criado com E noutro dispositivo) aqui:\n{0}",
    ]),
    // --- desktop: channel list
    ("type a channel name      Enter  done      Esc  clear", [
        "scrivi il nome di un canale      Invio  fatto      Esc  cancella",
        "escribe el nombre de un canal      Intro  listo      Esc  borrar",
        "tapez un nom de chaîne      Entrée  terminé      Échap  effacer",
        "Sendernamen eingeben      Enter  fertig      Esc  löschen",
        "escreva o nome de um canal      Enter  concluído      Esc  limpar",
    ]),
    ("/  search     c  countries     g / G  recent, groups     S  favorites", [
        "/  cerca     c  paesi     g / G  recenti, gruppi     S  preferiti",
        "/  buscar     c  países     g / G  recientes, grupos     S  favoritos",
        "/  rechercher     c  pays     g / G  récentes, groupes     S  favoris",
        "/  suchen     c  Länder     g / G  zuletzt, Gruppen     S  Favoriten",
        "/  pesquisar     c  países     g / G  recentes, grupos     S  favoritos",
    ]),
    ("No channels found", [
        "Nessun canale trovato",
        "No se encontraron canales",
        "Aucune chaîne trouvée",
        "Keine Sender gefunden",
        "Nenhum canal encontrado",
    ]),
    ("No channels here", [
        "Nessun canale qui",
        "No hay canales aquí",
        "Aucune chaîne ici",
        "Hier gibt es keine Sender",
        "Nenhum canal aqui",
    ]),
    ("{0}–{1} of {2}     {3}…", [
        "{0}–{1} di {2}     {3}…",
        "{0}–{1} de {2}     {3}…",
        "{0}–{1} sur {2}     {3}…",
        "{0}–{1} von {2}     {3}…",
        "{0}–{1} de {2}     {3}…",
    ]),
    ("{0}–{1} of {2}     showing: {3}", [
        "{0}–{1} di {2}     mostrati: {3}",
        "{0}–{1} de {2}     mostrando: {3}",
        "{0}–{1} sur {2}     affichage : {3}",
        "{0}–{1} von {2}     angezeigt: {3}",
        "{0}–{1} de {2}     a mostrar: {3}",
    ]),
    ("{0}–{1} of {2}     wheel scroll · click / Enter watch · Esc close", [
        "{0}–{1} di {2}     rotellina scorri · clic / Invio guarda · Esc chiudi",
        "{0}–{1} de {2}     rueda desplazar · clic / Intro ver · Esc cerrar",
        "{0}–{1} sur {2}     molette défiler · clic / Entrée regarder · Échap fermer",
        "{0}–{1} von {2}     Mausrad blättern · Klick / Enter ansehen · Esc schließen",
        "{0}–{1} de {2}     roda deslocar · clique / Enter ver · Esc fechar",
    ]),
    ("Countries", ["Paesi", "Países", "Pays", "Länder", "Países"]),
    ("Languages", ["Lingue", "Idiomas", "Langues", "Sprachen", "Idiomas"]),
    ("countries", ["paesi", "países", "pays", "Länder", "países"]),
    ("languages", ["lingue", "idiomas", "langues", "Sprachen", "idiomas"]),
    ("{0} · {1} chosen", [
        "{0} · {1} scelti",
        "{0} · {1} elegidos",
        "{0} · {1} choisis",
        "{0} · {1} ausgewählt",
        "{0} · {1} escolhidos",
    ]),
    ("type to find     Enter  tick     Tab  {0}     Del  clear all     Esc  back", [
        "scrivi per trovare     Invio  spunta     Tab  {0}     Canc  azzera tutto     Esc  indietro",
        "escribe para buscar     Intro  marcar     Tab  {0}     Supr  borrar todo     Esc  atrás",
        "tapez pour chercher     Entrée  cocher     Tab  {0}     Suppr  tout effacer     Échap  retour",
        "tippen zum Suchen     Enter  ankreuzen     Tab  {0}     Entf  alles löschen     Esc  zurück",
        "escreva para procurar     Enter  marcar     Tab  {0}     Del  limpar tudo     Esc  voltar",
    ]),
    ("Not available for this playlist", [
        "Non disponibile per questa playlist",
        "No disponible para esta lista",
        "Non disponible pour cette liste",
        "Für diese Senderliste nicht verfügbar",
        "Não disponível para esta lista",
    ]),
    ("Nothing matches", [
        "Nessun risultato",
        "No hay coincidencias",
        "Aucun résultat",
        "Keine Treffer",
        "Nenhum resultado",
    ]),
    ("✕  Clear all filters (show every channel)", [
        "✕  Azzera tutti i filtri (mostra tutti i canali)",
        "✕  Quitar todos los filtros (mostrar todos los canales)",
        "✕  Effacer tous les filtres (afficher toutes les chaînes)",
        "✕  Alle Filter löschen (alle Sender zeigen)",
        "✕  Limpar todos os filtros (mostrar todos os canais)",
    ]),
    ("showing channels from / in: {0}", [
        "canali mostrati da / in: {0}",
        "canales mostrados de / en: {0}",
        "chaînes affichées de / en : {0}",
        "angezeigte Sender aus / auf: {0}",
        "canais mostrados de / em: {0}",
    ]),
    ("nothing chosen: showing all channels", [
        "nessuna scelta: tutti i canali",
        "nada elegido: se muestran todos los canales",
        "rien de choisi : toutes les chaînes",
        "nichts ausgewählt: alle Sender",
        "nada escolhido: todos os canais",
    ]),
    // --- desktop: phone remote card
    ("Phone remote", [
        "Telecomando dal telefono",
        "Mando desde el móvil",
        "Télécommande sur téléphone",
        "Handy-Fernbedienung",
        "Comando no telemóvel",
    ]),
    ("This computer isn't connected to a network.", [
        "Questo computer non è connesso a una rete.",
        "Este ordenador no está conectado a una red.",
        "Cet ordinateur n'est connecté à aucun réseau.",
        "Dieser Computer ist mit keinem Netzwerk verbunden.",
        "Este computador não está ligado a uma rede.",
    ]),
    ("Scan with your phone's camera (same Wi-Fi as this computer),", [
        "Inquadra con la fotocamera del telefono (stesso Wi-Fi di questo computer),",
        "Escanéalo con la cámara del móvil (misma Wi-Fi que este ordenador),",
        "Scannez avec l'appareil photo du téléphone (même Wi-Fi que cet ordinateur),",
        "Mit der Handykamera scannen (gleiches WLAN wie dieser Computer),",
        "Leia com a câmara do telemóvel (mesma Wi-Fi que este computador),",
    ]),
    ("or open  {0}", ["oppure apri  {0}", "o abre  {0}", "ou ouvrez  {0}", "oder öffne  {0}", "ou abra  {0}"]),
    ("and enter the PIN  {0}", [
        "e inserisci il PIN  {0}",
        "e introduce el PIN  {0}",
        "et saisissez le code  {0}",
        "und gib die PIN ein:  {0}",
        "e introduza o PIN  {0}",
    ]),
    ("Any key closes this card. The remote keeps working.", [
        "Qualsiasi tasto chiude questo riquadro. Il telecomando continua a funzionare.",
        "Cualquier tecla cierra este cuadro. El mando sigue funcionando.",
        "Une touche ferme ce cadre. La télécommande continue de fonctionner.",
        "Eine beliebige Taste schließt diese Karte. Die Fernbedienung funktioniert weiter.",
        "Qualquer tecla fecha este cartão. O comando continua a funcionar.",
    ]),
];

/// Desktop key help: [English, Italian, Spanish, French, German, Portuguese].
const HELP: [&str; 6] = [
    "\
Up / Down / PgUp / PgDn / wheel   change channel
0-9 then wait or Enter             go to channel number
Backspace                          previous channel
Left / Right                       volume      m  mute
i  info      l / Tab  channel panel (wheel, click, Esc)
/ or Ctrl+F  search channels by name
c            choose countries and languages
a  audio language      t  subtitles   (when offered)
r            phone remote (QR code to scan)
E / I        export / import favorites (Downloads folder)
g / G        recently watched, groups
s  toggle favorite    S  favorites only
p  small picture: a second channel in the corner
[ ]  its channel      x  swap the two channels
f  fullscreen         space  pause      q  quit",
    "\
Su / Giù / PagSu / PagGiù / rotellina   cambia canale
0-9 poi attendi o Invio                 vai al numero di canale
Backspace                               canale precedente
Sinistra / Destra                       volume      m  muto
i  info      l / Tab  elenco canali (rotellina, clic, Esc)
/ o Ctrl+F   cerca i canali per nome
c            scegli paesi e lingue
a  lingua audio        t  sottotitoli   (se disponibili)
r            telecomando dal telefono (codice QR)
E / I        esporta / importa preferiti (cartella Download)
g / G        visti di recente, gruppi
s  aggiungi/togli preferito    S  solo preferiti
p  riquadro: un secondo canale nell'angolo
[ ]  il suo canale    x  scambia i due canali
f  schermo intero     spazio  pausa      q  esci",
    "\
Arriba / Abajo / RePág / AvPág / rueda   cambiar de canal
0-9 y espera o Intro                     ir al número de canal
Retroceso                                canal anterior
Izquierda / Derecha                      volumen      m  silencio
i  info      l / Tab  lista de canales (rueda, clic, Esc)
/ o Ctrl+F   buscar canales por nombre
c            elegir países e idiomas
a  idioma del audio    t  subtítulos   (si los hay)
r            mando desde el móvil (código QR)
E / I        exportar / importar favoritos (carpeta Descargas)
g / G        vistos recientemente, grupos
s  marcar/quitar favorito    S  solo favoritos
p  imagen pequeña: un segundo canal en la esquina
[ ]  su canal         x  intercambiar los dos canales
f  pantalla completa  espacio  pausa      q  salir",
    "\
Haut / Bas / PgPréc / PgSuiv / molette   changer de chaîne
0-9 puis attendre ou Entrée              aller au numéro de chaîne
Retour arrière                           chaîne précédente
Gauche / Droite                          volume      m  muet
i  infos     l / Tab  liste des chaînes (molette, clic, Échap)
/ ou Ctrl+F  rechercher une chaîne par son nom
c            choisir pays et langues
a  langue audio        t  sous-titres   (si proposés)
r            télécommande sur téléphone (code QR)
E / I        exporter / importer les favoris (dossier Téléchargements)
g / G        regardées récemment, groupes
s  ajouter/retirer des favoris    S  favoris seulement
p  petite image : une deuxième chaîne dans le coin
[ ]  sa chaîne        x  échanger les deux chaînes
f  plein écran        espace  pause      q  quitter",
    "\
Hoch / Runter / Bild↑ / Bild↓ / Mausrad   Sender wechseln
0-9, dann warten oder Enter              zur Sendernummer
Rücktaste                                vorheriger Sender
Links / Rechts                           Lautstärke      m  stumm
i  Info      l / Tab  Senderliste (Mausrad, Klick, Esc)
/ oder Strg+F  Sender nach Namen suchen
c            Länder und Sprachen wählen
a  Audiosprache        t  Untertitel   (falls vorhanden)
r            Handy-Fernbedienung (QR-Code)
E / I        Favoriten exportieren / importieren (Ordner Downloads)
g / G        zuletzt gesehen, Gruppen
s  Favorit an/aus     S  nur Favoriten
p  kleines Bild: ein zweiter Sender in der Ecke
[ ]  sein Sender      x  die beiden Sender tauschen
f  Vollbild           Leertaste  Pause      q  beenden",
    "\
Cima / Baixo / PgUp / PgDn / roda   mudar de canal
0-9 e esperar ou Enter              ir para o número do canal
Backspace                           canal anterior
Esquerda / Direita                  volume      m  sem som
i  info      l / Tab  lista de canais (roda, clique, Esc)
/ ou Ctrl+F  pesquisar canais pelo nome
c            escolher países e idiomas
a  idioma do áudio     t  legendas   (quando existirem)
r            comando no telemóvel (código QR)
E / I        exportar / importar favoritos (pasta Transferências)
g / G        vistos recentemente, grupos
s  marcar/desmarcar favorito    S  só favoritos
p  imagem pequena: um segundo canal no canto
[ ]  o seu canal      x  trocar os dois canais
f  ecrã inteiro       espaço  pausa      q  sair",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(s: &str) -> Vec<String> {
        let mut v: Vec<String> = (0..9)
            .map(|i| format!("{{{i}}}"))
            .filter(|p| s.contains(p.as_str()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn every_translation_is_complete() {
        for (en, tr) in TABLE {
            for (i, s) in tr.iter().enumerate() {
                assert!(!s.is_empty(), "missing translation {i} of {en:?}");
                assert_eq!(
                    placeholders(en),
                    placeholders(s),
                    "placeholders differ in {s:?}"
                );
                assert_eq!(
                    en.matches('\n').count(),
                    s.matches('\n').count(),
                    "line breaks differ in {s:?}"
                );
            }
        }
        let mut keys: Vec<&str> = TABLE.iter().map(|(en, _)| *en).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), TABLE.len(), "duplicate messages");
    }

    /// Every message the code asks for has translations.
    #[test]
    fn code_uses_only_known_messages() {
        let sources = [
            include_str!("tuner.rs"),
            include_str!("favorites.rs"),
            include_str!("../../iptv-desktop/src/main.rs"),
            include_str!("../../iptv-desktop/src/panel.rs"),
        ];
        for src in sources {
            for call in ["t(\"", "tf(\""] {
                for (pos, _) in src.match_indices(call) {
                    // Only real calls, not words ending in "t(".
                    let before = src[..pos].chars().last().unwrap_or(' ');
                    if before.is_alphanumeric() || before == '_' {
                        continue;
                    }
                    let rest = &src[pos + call.len()..];
                    let Some(end) = rest.find('"') else { continue };
                    let text = rest[..end].replace("\\n", "\n");
                    assert!(
                        TABLE.iter().any(|(en, _)| *en == text),
                        "no translations for {text:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn picks_language_and_translates() {
        // (The global language isn't changed here: tests run in parallel.)
        assert_eq!(Lang::from_code("it_IT.UTF-8"), Lang::It);
        assert_eq!(Lang::from_code("pt-BR"), Lang::Pt);
        assert_eq!(Lang::from_code("ja-JP"), Lang::En);
        assert_eq!(t_in(Lang::It, "All channels"), "Tutti i canali");
        assert_eq!(
            t_in(Lang::De, "No channel {0}").replace("{0}", "42"),
            "Kein Sender 42"
        );
        assert_eq!(t_in(Lang::Fr, "not in the table"), "not in the table");
        assert_eq!(t_in(Lang::En, "All channels"), "All channels");
    }
}
