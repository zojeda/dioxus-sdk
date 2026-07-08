package dev.dioxus.push

import android.app.Activity

/**
 * Empty bridge class instantiated by the Rust wrapper that `#[manganis::ffi]`
 * generates (`env.new_object("dev/dioxus/push/DioxusPushKotlin",
 * "(Landroid/app/Activity;)V", activity)`).
 *
 * Its real purpose is the plugin metadata the macro embeds alongside it: `dx`
 * (>= 0.7.9) reads that metadata from the compiled Rust library and bundles
 * this Kotlin module into the generated Android project automatically. All
 * actual push traffic flows through [DioxusPush] via the hand-written JNI
 * bindings in `src/platform/android.rs`.
 */
@Suppress("unused", "UNUSED_PARAMETER")
class DioxusPushKotlin(activity: Activity)
