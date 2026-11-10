//! JNI bridge for the Android app (`io.github.vickdini.iptv.Core`).
//!
//! The Android side is a thin shell (UI + ExoPlayer). All TV behaviour lives
//! here, shared with the desktop app: the [`Tuner`] state machine, playlist
//! download with conditional requests, and background channel checks.
//!
//! Data crosses the boundary as JSON strings to keep the JNI surface small.
//! Errors become Java `RuntimeException`s; panics are caught so they never
//! unwind into the JVM.

mod app;

use jni::{
    objects::{JClass, JObject, JString},
    sys::{jint, jintArray, jobject, jobjectArray, jstring},
    JNIEnv,
};

pub use app::App;

/// Runs `f`, converting errors and panics into a Java exception (returning
/// null) and the success value into a Java string.
fn call<'l>(env: &mut JNIEnv<'l>, f: impl FnOnce(&mut JNIEnv<'l>) -> anyhow::Result<String>) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(env)));
    let message = match result {
        Ok(Ok(s)) => match env.new_string(s) {
            Ok(js) => return js.into_raw(),
            Err(e) => format!("could not create string: {e}"),
        },
        Ok(Err(e)) => format!("{e:#}"),
        Err(_) => "internal error (panic) in iptv core".to_string(),
    };
    let _ = env.throw_new("java/lang/RuntimeException", message);
    std::ptr::null_mut()
}

fn string(env: &mut JNIEnv, s: &JString) -> anyhow::Result<Option<String>> {
    if s.is_null() {
        return Ok(None);
    }
    Ok(Some(env.get_string(s)?.into()))
}

macro_rules! export {
    ($name:ident, |$env:ident $(, $arg:ident : $ty:ty)*| $body:expr) => {
        #[no_mangle]
        pub extern "system" fn $name<'l>(mut env: JNIEnv<'l>, _class: JClass<'l> $(, $arg: $ty)*) -> jstring {
            call(&mut env, |$env| $body)
        }
    };
}

// init(cacheDir, configDir): String  -- must be called first.
export!(Java_io_github_vickdini_iptv_Core_init, |env, cache_dir: JString<'l>, config_dir: JString<'l>| {
    let cache = string(env, &cache_dir)?.unwrap_or_default();
    let config = string(env, &config_dir)?.unwrap_or_default();
    app::init(cache.into(), config.into());
    Ok("{}".into())
});

// loadPlaylist(url): {"channels":n,"freshness":"..."}  -- blocking, call off the main thread.
export!(Java_io_github_vickdini_iptv_Core_loadPlaylist, |env, url: JString<'l>| {
    let url = string(env, &url)?.unwrap_or_default();
    app::load_playlist(&url)
});

// start(): effects
export!(Java_io_github_vickdini_iptv_Core_start, |_env| app::with(|a| Ok(a.start())));

// action(name, arg?): effects. Names as on desktop ("up", "down", "digit" "7", "ok",
// "prev", "info", "group-next", "group-prev", "fav", "fav-only", "help") plus
// "digit-timeout".
export!(Java_io_github_vickdini_iptv_Core_action, |env, name: JString<'l>, arg: JString<'l>| {
    let name = string(env, &name)?.unwrap_or_default();
    let arg = string(env, &arg)?;
    app::with(|a| Ok(a.action(&name, arg.as_deref())))
});

// tuneTo(idx): effects
export!(Java_io_github_vickdini_iptv_Core_tuneTo, |_env, idx: jint| app::with(|a| Ok(a.tune_to(idx as usize))));

// toggleFavorite(idx): effects
export!(Java_io_github_vickdini_iptv_Core_toggleFavorite, |_env, idx: jint| {
    app::with(|a| Ok(a.toggle_favorite(idx as usize)))
});

// playbackFailed(seq): effects -- the player reported an error for tune #seq.
export!(Java_io_github_vickdini_iptv_Core_playbackFailed, |_env, seq: jint| {
    app::with(|a| Ok(a.playback_failed(seq as u64)))
});

// playbackStarted(seq): "{}" -- tune #seq is playing; remember it works.
export!(Java_io_github_vickdini_iptv_Core_playbackStarted, |_env, seq: jint| {
    app::with(|a| {
        a.playback_started(seq as u64);
        Ok("{}".into())
    })
});

// checkUpdate(currentVersion): {} if up to date, else
// {"version","fileName","url","sha256"} for the newer APK. Blocking (network).
export!(Java_io_github_vickdini_iptv_Core_checkUpdate, |env, current: JString<'l>| {
    let current = string(env, &current)?.unwrap_or_default();
    match iptv_net::update::check(iptv_net::update::RELEASES, "IPTV-", ".apk", &current)? {
        None => Ok("{}".into()),
        Some(u) => Ok(serde_json::json!({
            "version": u.version, "fileName": u.file_name, "url": u.url, "sha256": u.sha256,
        })
        .to_string()),
    }
});

// downloadUpdate(updateJson, destPath): downloads the APK and verifies its
// SHA-256. Blocking. Throws on failure.
export!(Java_io_github_vickdini_iptv_Core_downloadUpdate, |env, update: JString<'l>, dest: JString<'l>| {
    let update = string(env, &update)?.unwrap_or_default();
    let dest = string(env, &dest)?.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&update)?;
    let field = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    let u = iptv_net::update::Update {
        version: field("version"),
        file_name: field("fileName"),
        url: field("url"),
        sha256: field("sha256"),
    };
    iptv_net::update::download(&u, std::path::Path::new(&dest))?;
    Ok("{}".into())
});

// setSearch(text): "{}" -- show only channels whose name matches (empty: all).
export!(Java_io_github_vickdini_iptv_Core_setSearch, |env, text: JString<'l>| {
    let text = string(env, &text)?.unwrap_or_default();
    app::with(|a| {
        a.set_search(&text);
        Ok("{}".into())
    })
});

// isLoaded(): "true"/"false" -- a playlist is already loaded in this process.
export!(Java_io_github_vickdini_iptv_Core_isLoaded, |_env| Ok(app::is_loaded().to_string()));

// searchText(): the search in effect, or "".
export!(Java_io_github_vickdini_iptv_Core_searchText, |_env| app::with(|a| Ok(a.search_text())));

// regionsInfo(): countries / languages with channel counts and the current choice.
export!(Java_io_github_vickdini_iptv_Core_regionsInfo, |_env| app::with(|a| Ok(a.regions_info())));

// setRegions(countriesJson, languagesJson): effects -- JSON arrays of names; both empty = all channels.
export!(Java_io_github_vickdini_iptv_Core_setRegions, |env, countries: JString<'l>, languages: JString<'l>| {
    let parse = |s: Option<String>| -> anyhow::Result<Vec<String>> {
        Ok(serde_json::from_str(&s.unwrap_or_else(|| "[]".into()))?)
    };
    let countries = parse(string(env, &countries)?)?;
    let languages = parse(string(env, &languages)?)?;
    app::with(|a| Ok(a.set_regions(countries, languages)))
});

// listInfo(): {"title":..., "current":idx, "status":...}
export!(Java_io_github_vickdini_iptv_Core_listInfo, |_env| app::with(|a| Ok(a.list_info())));

/// Like [`call`] for functions returning a Java array (or any object).
fn call_obj<'l>(
    env: &mut JNIEnv<'l>,
    f: impl FnOnce(&mut JNIEnv<'l>) -> anyhow::Result<jobject>,
) -> jobject {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(env)));
    let message = match result {
        Ok(Ok(obj)) => return obj,
        Ok(Err(e)) => format!("{e:#}"),
        Err(_) => "internal error (panic) in iptv core".to_string(),
    };
    let _ = env.throw_new("java/lang/RuntimeException", message);
    std::ptr::null_mut()
}

fn int_array(env: &mut JNIEnv, values: &[i32]) -> anyhow::Result<jobject> {
    let arr = env.new_int_array(values.len() as i32)?;
    env.set_int_array_region(&arr, 0, values)?;
    Ok(arr.into_raw())
}

/// view(): IntArray -- channel indices in the current view (cheap; call on every refresh).
#[no_mangle]
pub extern "system" fn Java_io_github_vickdini_iptv_Core_view<'l>(mut env: JNIEnv<'l>, _class: JClass<'l>) -> jintArray {
    call_obj(&mut env, |env| {
        let v = app::with(|a| Ok(a.view_indices()))?;
        int_array(env, &v)
    })
}

/// favorites(): IntArray -- indices of favorite channels.
#[no_mangle]
pub extern "system" fn Java_io_github_vickdini_iptv_Core_favorites<'l>(mut env: JNIEnv<'l>, _class: JClass<'l>) -> jintArray {
    call_obj(&mut env, |env| {
        let v = app::with(|a| Ok(a.favorite_indices()))?;
        int_array(env, &v)
    })
}

/// names(): Array<String> -- every channel name by index (call once, off the main thread).
#[no_mangle]
pub extern "system" fn Java_io_github_vickdini_iptv_Core_names<'l>(mut env: JNIEnv<'l>, _class: JClass<'l>) -> jobjectArray {
    call_obj(&mut env, |env| {
        let names = app::with(|a| Ok(a.names()))?;
        let arr = env.new_object_array(names.len() as i32, "java/lang/String", JObject::null())?;
        for (i, name) in names.iter().enumerate() {
            let s = env.new_string(name)?;
            env.set_object_array_element(&arr, i as i32, &s)?;
            env.delete_local_ref(s)?;
        }
        Ok(arr.into_raw())
    })
}

// poll(): {"changed":bool, "status":...|null}  -- merge background check results.
export!(Java_io_github_vickdini_iptv_Core_poll, |_env| app::with(|a| Ok(a.poll())));

// save(): "{}"  -- persist state and check results (call from onStop).
export!(Java_io_github_vickdini_iptv_Core_save, |_env| app::with(|a| {
    a.save();
    Ok("{}".into())
}));
