package dev.opendesk.opendesk_mobile

import android.Manifest
import android.app.Activity
import android.app.NotificationManager
import android.content.Intent
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.util.concurrent.atomic.AtomicBoolean

/** The Flutter host calls start only after accepting a peer locally. */
class MainActivity : FlutterActivity() {
    private var pendingStart: MethodChannel.Result? = null
    private val captureRequest = 4101
    private val notificationRequest = 4102

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        val channel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "dev.opendesk/capture")
        CaptureBridge.channel = channel
        channel.setMethodCallHandler { call, result ->
            when (call.method) {
                "start" -> {
                    if (pendingStart != null || CaptureBridge.running.get()) {
                        result.error("ALREADY_STARTED", "Screen sharing is already active or awaiting approval", null)
                    } else {
                        pendingStart = result
                        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), notificationRequest)
                        } else {
                            requestScreenConsent()
                        }
                    }
                }
                "stop" -> {
                    pendingStart?.error("CANCELED", "Screen sharing was canceled", null)
                    pendingStart = null
                    stopService(Intent(this, CaptureService::class.java))
                    result.success(null)
                }
                else -> result.notImplemented()
            }
        }
    }

    private fun requestScreenConsent() {
        if (pendingStart == null) return
        if (Build.VERSION.SDK_INT >= 24 && !getSystemService(NotificationManager::class.java).areNotificationsEnabled()) {
            pendingStart?.error("NOTIFICATIONS_DISABLED", "Enable Opendesk notifications so sharing has a visible Stop action", null)
            pendingStart = null
            return
        }
        try {
            val manager = getSystemService(MediaProjectionManager::class.java)
            @Suppress("DEPRECATION")
            startActivityForResult(manager.createScreenCaptureIntent(), captureRequest)
        } catch (error: Exception) {
            pendingStart?.error("CAPTURE_UNAVAILABLE", error.message, null)
            pendingStart = null
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != notificationRequest) return
        if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
            requestScreenConsent()
        } else {
            pendingStart?.error("NOTIFICATION_PERMISSION_DENIED", "Notifications are required for the visible screen-sharing Stop action", null)
            pendingStart = null
        }
    }

    @Deprecated("Android activity-result callback used for FlutterActivity compatibility")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != captureRequest) return
        val result = pendingStart ?: return // A locally canceled request never starts capture.
        pendingStart = null
        if (resultCode != Activity.RESULT_OK || data == null) {
            result.error("CONSENT_DENIED", "Android screen sharing was not approved", null)
            return
        }
        try {
            val service = Intent(this, CaptureService::class.java)
                .putExtra(CaptureService.RESULT_CODE, resultCode)
                .putExtra(CaptureService.RESULT_DATA, data)
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(service) else startService(service)
            result.success(null)
        } catch (error: Exception) {
            result.error("CAPTURE_START_FAILED", error.message, null)
        }
    }

    override fun cleanUpFlutterEngine(flutterEngine: FlutterEngine) {
        pendingStart?.error("ACTIVITY_CLOSED", "Opendesk was closed", null)
        pendingStart = null
        stopService(Intent(this, CaptureService::class.java))
        CaptureBridge.channel?.setMethodCallHandler(null)
        CaptureBridge.channel = null
        super.cleanUpFlutterEngine(flutterEngine)
    }
}

/** At most one frame is waiting for Dart, so a paused UI cannot grow a queue. */
object CaptureBridge {
    @Volatile var channel: MethodChannel? = null
    val running = AtomicBoolean(false)
    private val framePending = AtomicBoolean(false)
    private val main = Handler(Looper.getMainLooper())
    fun frame(jpeg: ByteArray) {
        if (!running.get() || !framePending.compareAndSet(false, true)) return
        main.post {
            val target = channel
            if (target == null || !running.get()) { framePending.set(false); return@post }
            target.invokeMethod("frame", jpeg, object : MethodChannel.Result {
                override fun success(result: Any?) { framePending.set(false) }
                override fun error(code: String, message: String?, details: Any?) { framePending.set(false) }
                override fun notImplemented() { framePending.set(false) }
            })
        }
    }
    fun stopped(reason: String) {
        running.set(false)
        framePending.set(false)
        main.post { channel?.invokeMethod("stopped", reason) }
    }
}
