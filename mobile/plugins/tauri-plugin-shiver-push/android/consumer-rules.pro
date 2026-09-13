# The manifest receiver is only ever named from AndroidManifest.xml, so R8 cannot see it being used
# and would otherwise strip it — which would silently remove the whole cold-start path.
-keep class com.shiver.push.PushReceiver { *; }
