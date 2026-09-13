# Rules every app using this plugin needs, applied to that app's own minified build.
#
# Nothing of this library's own has to be kept: its only reflective entry points are the Tauri
# plugin annotations, and the app already keeps those.
#
# What does have to be said is about Tink, which arrives here with
# `androidx.security:security-crypto` — the library behind EncryptedSharedPreferences. Tink is
# compiled against JSR-305 annotations that are not on the Android runtime classpath and are not
# needed there, so R8 finds the references dangling and fails the release build outright. They are
# annotations only; suppressing the warning is the whole fix, and R8 itself generates exactly these
# two lines in `missing_rules.txt`.
-dontwarn javax.annotation.Nullable
-dontwarn javax.annotation.concurrent.GuardedBy
