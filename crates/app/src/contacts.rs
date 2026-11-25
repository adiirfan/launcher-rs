//! Looking up contacts on the phone for search. Names never go to the log.

use android_activity::AndroidApp;
use jni::objects::{JObject, JString, JValue};
use jni::{Env, jni_sig, jni_str};

use crate::jni_util::with_activity;

const PERMISSION: &str = "android.permission.READ_CONTACTS";

#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub name: String,
    pub lookup_uri: String,
}

pub fn has_permission(app: &AndroidApp) -> bool {
    with_activity(app, |env, activity| {
        let p = env.new_string(PERMISSION)?;
        env.call_method(activity, jni_str!("checkSelfPermission"), jni_sig!("(Ljava/lang/String;)I"), &[JValue::Object(&p)])?
            .i()
    })
    .map(|r| r == 0)
    .unwrap_or(false)
}

/// After a refusal: true means Android will still show its question again;
/// false means it won't ("don't ask again"), so only Settings can turn it on.
pub fn can_ask_again(app: &AndroidApp) -> bool {
    with_activity(app, |env, activity| {
        let p = env.new_string(PERMISSION)?;
        env.call_method(
            activity,
            jni_str!("shouldShowRequestPermissionRationale"),
            jni_sig!("(Ljava/lang/String;)Z"),
            &[JValue::Object(&p)],
        )?
        .z()
    })
    .unwrap_or(false)
}

fn string_at(env: &mut Env, cursor: &JObject, col: i32) -> jni::errors::Result<String> {
    let s = env.call_method(cursor, jni_str!("getString"), jni_sig!("(I)Ljava/lang/String;"), &[JValue::Int(col)])?.l()?;
    if s.is_null() {
        return Ok(String::new());
    }
    let s = env.cast_local::<JString>(s)?;
    Ok(s.mutf8_chars(env)?.into())
}

/// Contacts whose name matches `query`, as the contacts app orders them.
pub fn search(app: &AndroidApp, query: &str, limit: usize) -> Vec<Contact> {
    let result = with_activity(app, |env, activity| {
        let resolver = env
            .call_method(activity, jni_str!("getContentResolver"), jni_sig!("()Landroid/content/ContentResolver;"), &[])?
            .l()?;
        let base = env
            .get_static_field(jni_str!("android/provider/ContactsContract$Contacts"), jni_str!("CONTENT_FILTER_URI"), jni_sig!("Landroid/net/Uri;"))?
            .l()?;
        let q = env.new_string(query.trim())?;
        let encoded = env
            .call_static_method(jni_str!("android/net/Uri"), jni_str!("encode"), jni_sig!("(Ljava/lang/String;)Ljava/lang/String;"), &[JValue::Object(&q)])?
            .l()?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("withAppendedPath"),
                jni_sig!("(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::Object(&base), JValue::Object(&encoded)],
            )?
            .l()?;
        let string_class = env.find_class(jni_str!("java/lang/String"))?;
        let projection = env.new_object_array(3, &string_class, &JObject::null())?;
        for (i, col) in ["_id", "lookup", "display_name"].into_iter().enumerate() {
            let s = env.new_string(col)?;
            projection.set_element(env, i, &s)?;
        }
        let null = JObject::null();
        let cursor = env
            .call_method(
                &resolver,
                jni_str!("query"),
                jni_sig!("(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"),
                &[JValue::Object(&uri), JValue::Object(&projection), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null)],
            )?
            .l()?;
        if cursor.is_null() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let rows = (|| -> jni::errors::Result<()> {
            while out.len() < limit && env.call_method(&cursor, jni_str!("moveToNext"), jni_sig!("()Z"), &[])?.z()? {
                let contact = env.with_local_frame(8, |env| -> jni::errors::Result<Contact> {
                    let id = env.call_method(&cursor, jni_str!("getLong"), jni_sig!("(I)J"), &[JValue::Int(0)])?.j()?;
                    let key = string_at(env, &cursor, 1)?;
                    let name = string_at(env, &cursor, 2)?;
                    let key_s = env.new_string(&key)?;
                    let lookup = env
                        .call_static_method(
                            jni_str!("android/provider/ContactsContract$Contacts"),
                            jni_str!("getLookupUri"),
                            jni_sig!("(JLjava/lang/String;)Landroid/net/Uri;"),
                            &[JValue::Long(id), JValue::Object(&key_s)],
                        )?
                        .l()?;
                    let lookup = env.call_method(&lookup, jni_str!("toString"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
                    let lookup = env.cast_local::<JString>(lookup)?;
                    Ok(Contact { name, lookup_uri: lookup.mutf8_chars(env)?.into() })
                })?;
                out.push(contact);
            }
            Ok(())
        })();
        // Always close the cursor, even if reading a row failed.
        let _ = env.call_method(&cursor, jni_str!("close"), jni_sig!("()V"), &[]);
        rows?;
        Ok(out)
    });
    match result {
        Ok(v) => {
            log::info!("contacts: {} match(es)", v.len());
            v
        }
        Err(e) => {
            log::warn!("contacts search failed: {e}");
            Vec::new()
        }
    }
}
