package dev.opendesk.opendesk_mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.SystemClock
import android.util.DisplayMetrics
import android.view.WindowManager
import java.io.ByteArrayOutputStream
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.max
import kotlin.math.roundToInt

/** Visible, consent-based screen sharing only. No accessibility/input injection. */
class CaptureService : Service() {
    companion object {
        const val RESULT_CODE = "capture_result_code"
        const val RESULT_DATA = "capture_result_data"
        private const val STOP = "dev.opendesk.STOP_CAPTURE"
        private const val CHANNEL = "opendesk_screen_sharing"
        private const val NOTIFICATION_ID = 4103
    }
    private lateinit var thread: HandlerThread
    private lateinit var worker: Handler
    private val closing = AtomicBoolean(false)
    private var projection: MediaProjection? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var reader: ImageReader? = null
    private var width = 0
    private var height = 0
    private var lastFrame = 0L
    private var stopReason = "Screen sharing stopped"
    private val projectionCallback = object : MediaProjection.Callback() {
        override fun onStop() { finishCapture("Android stopped screen sharing") }
        override fun onCapturedContentResize(width: Int, height: Int) {
            if (width > 0 && height > 0 && !closing.get()) resize(width, height)
        }
    }

    override fun onCreate() {
        super.onCreate()
        thread = HandlerThread("Opendesk screen capture").also { it.start() }
        worker = Handler(thread.looper)
    }
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == STOP) { finishCapture("Screen sharing stopped from notification"); return START_NOT_STICKY }
        if (intent == null || CaptureBridge.running.get()) { stopSelf(); return START_NOT_STICKY }
        try {
            val notification = sharingNotification()
            if (Build.VERSION.SDK_INT >= 29) startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
            else startForeground(NOTIFICATION_ID, notification)
            val data = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(RESULT_DATA, Intent::class.java)
                else { @Suppress("DEPRECATION") intent.getParcelableExtra<Intent>(RESULT_DATA) }
            if (data == null) throw IllegalArgumentException("Missing Android capture consent")
            val resultCode = intent.getIntExtra(RESULT_CODE, 0)
            worker.post {
                try {
                    if (closing.get()) return@post
                    val manager = getSystemService(MediaProjectionManager::class.java)
                    projection = manager.getMediaProjection(resultCode, data)
                    projection!!.registerCallback(projectionCallback, worker)
                    val dimensions = displayDimensions()
                    val size = boundedSize(dimensions.first, dimensions.second)
                    width = size.first; height = size.second
                    reader = createReader(width, height)
                    virtualDisplay = projection!!.createVirtualDisplay("Opendesk sharing", width, height,
                        resources.configuration.densityDpi, DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
                        reader!!.surface, null, worker)
                    CaptureBridge.running.set(true)
                } catch (error: Exception) { finishCapture("Screen capture failed: ${error.message}") }
            }
        } catch (error: Exception) { finishCapture("Screen capture failed: ${error.message}") }
        return START_NOT_STICKY // A restart cannot reuse an expired projection token.
    }

    private fun sharingNotification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) manager.createNotificationChannel(NotificationChannel(CHANNEL, "Screen sharing", NotificationManager.IMPORTANCE_LOW))
        val immutable = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val stop = PendingIntent.getService(this, 4104, Intent(this, CaptureService::class.java).setAction(STOP), immutable)
        val open = PendingIntent.getActivity(this, 4105, Intent(this, MainActivity::class.java), immutable)
        val builder = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this, CHANNEL) else {
            @Suppress("DEPRECATION") Notification.Builder(this)
        }
        return builder.setContentTitle("Opendesk screen sharing")
            .setContentText("Your screen is being shared. Tap Stop to end sharing.")
            .setSmallIcon(android.R.drawable.ic_menu_view).setOngoing(true)
            .setCategory(Notification.CATEGORY_SERVICE).setContentIntent(open)
            .addAction(Notification.Action.Builder(android.R.drawable.ic_menu_close_clear_cancel, "Stop sharing", stop).build())
            .build()
    }

    private fun displayDimensions(): Pair<Int, Int> {
        val manager = getSystemService(WindowManager::class.java)
        if (Build.VERSION.SDK_INT >= 30) {
            val bounds = manager.maximumWindowMetrics.bounds
            return Pair(bounds.width(), bounds.height())
        }
        val metrics = DisplayMetrics()
        @Suppress("DEPRECATION") manager.defaultDisplay.getRealMetrics(metrics)
        return Pair(metrics.widthPixels, metrics.heightPixels)
    }
    private fun boundedSize(w: Int, h: Int): Pair<Int, Int> {
        require(w > 0 && h > 0) { "Invalid display size" }
        val scale = (1280.0 / max(w, h)).coerceAtMost(1.0)
        return Pair((w * scale).roundToInt().coerceAtLeast(1), (h * scale).roundToInt().coerceAtLeast(1))
    }
    private fun createReader(w: Int, h: Int): ImageReader {
        val next = ImageReader.newInstance(w, h, PixelFormat.RGBA_8888, 2)
        next.setOnImageAvailableListener({ source ->
            val image = try { source.acquireLatestImage() } catch (_: IllegalStateException) { null }
            if (image != null) {
                try {
                    val now = SystemClock.elapsedRealtime()
                    if (!closing.get() && CaptureBridge.running.get() && now - lastFrame >= 166L) {
                        lastFrame = now
                        val plane = image.planes[0]
                        val stride = plane.pixelStride
                        val padding = plane.rowStride - stride * image.width
                        require(stride == 4 && padding >= 0) { "Unsupported capture pixel layout" }
                        require(image.width <= 1280 && image.height <= 1280) { "Capture buffer exceeds limit" }
                        // The final row can omit its padding. Copy each real row,
                        // rather than asking Bitmap to read stride*height bytes.
                        val pixels = ByteArray(image.width * image.height * 4)
                        val sourceBuffer = plane.buffer.duplicate()
                        val rowBytes = image.width * 4
                        for (row in 0 until image.height) {
                            sourceBuffer.position(row * plane.rowStride)
                            sourceBuffer.get(pixels, row * rowBytes, rowBytes)
                        }
                        val bitmap = Bitmap.createBitmap(image.width, image.height, Bitmap.Config.ARGB_8888)
                        try {
                            bitmap.copyPixelsFromBuffer(java.nio.ByteBuffer.wrap(pixels))
                            val encoded = ByteArrayOutputStream()
                            bitmap.compress(Bitmap.CompressFormat.JPEG, 65, encoded)
                            if (encoded.size() <= 4 * 1024 * 1024) CaptureBridge.frame(encoded.toByteArray())
                        } finally { bitmap.recycle() }
                    }
                } catch (error: Exception) { finishCapture("Screen frame failed: ${error.message}") }
                finally { image.close() }
            }
        }, worker)
        return next
    }
    private fun resize(w: Int, h: Int) {
        if (closing.get()) return
        val size = boundedSize(w, h)
        if (size.first == width && size.second == height || virtualDisplay == null) return
        try {
            val next = createReader(size.first, size.second)
            virtualDisplay?.surface = null
            virtualDisplay?.resize(size.first, size.second, resources.configuration.densityDpi)
            virtualDisplay?.surface = next.surface
            val old = reader; reader = next; width = size.first; height = size.second; old?.close()
        } catch (error: Exception) { finishCapture("Screen resize failed: ${error.message}") }
    }
    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (Build.VERSION.SDK_INT < 34) worker.post {
            val dimensions = displayDimensions(); resize(dimensions.first, dimensions.second)
        }
    }
    private fun finishCapture(reason: String) {
        if (!closing.compareAndSet(false, true)) return
        stopReason = reason
        CaptureBridge.running.set(false)
        stopSelf()
    }
    override fun onDestroy() {
        closing.set(true)
        CaptureBridge.stopped(stopReason)
        worker.post {
            virtualDisplay?.release(); virtualDisplay = null
            reader?.close(); reader = null
            projection?.unregisterCallback(projectionCallback)
            projection?.stop(); projection = null
            thread.quitSafely()
        }
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }
}

