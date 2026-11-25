package com.iconnect.app;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.DashPathEffect;
import android.graphics.Paint;
import android.util.AttributeSet;
import android.view.HapticFeedbackConstants;
import android.view.MotionEvent;
import android.view.View;

public class TouchpadView extends View {
    private float lastX, lastY;
    private float touchDownX, touchDownY;
    private long touchDownTime;
    private boolean isMoving = false;
    private boolean isMultiTouch = false;
    private float sensitivity = 1.3f;

    private float lastScrollX, lastScrollY;
    private long twoFingerDownTime;
    private boolean hasScrolled = false;

    private long lastTapUpTime = 0;
    private float lastTapUpX = 0, lastTapUpY = 0;
    private boolean isDragging = false;

    private Paint touchPaint;
    private Paint touchRingPaint;
    private Paint dragPaint;
    private Paint textPaint;
    private Paint borderPaint;
    private float touchX = -1, touchY = -1;
    private boolean isTouching = false;

    public TouchpadView(Context context) {
        super(context);
        init();
    }

    public TouchpadView(Context context, AttributeSet attrs) {
        super(context, attrs);
        init();
    }

    private void init() {
        touchPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
        touchPaint.setColor(Color.parseColor("#44B6FF00"));
        touchPaint.setStyle(Paint.Style.FILL);

        touchRingPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
        touchRingPaint.setColor(Color.parseColor("#CCB6FF00"));
        touchRingPaint.setStyle(Paint.Style.STROKE);
        touchRingPaint.setStrokeWidth(3.0f);

        dragPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
        dragPaint.setColor(Color.parseColor("#CCFFB020"));
        dragPaint.setStyle(Paint.Style.STROKE);
        dragPaint.setStrokeWidth(5.0f);

        textPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
        textPaint.setColor(Color.parseColor("#FFB020"));
        textPaint.setTextSize(34f);
        textPaint.setTextAlign(Paint.Align.CENTER);
        textPaint.setFakeBoldText(true);

        borderPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
        borderPaint.setColor(Color.parseColor("#2A4219"));
        borderPaint.setStyle(Paint.Style.STROKE);
        borderPaint.setStrokeWidth(2.5f);
        borderPaint.setPathEffect(new DashPathEffect(new float[]{10, 10}, 0));
    }

    public void setSensitivity(float s) {
        this.sensitivity = Math.max(0.5f, Math.min(3.0f, s));
    }

    @Override
    protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);

        int w = getWidth();
        int h = getHeight();

        canvas.drawLine(w / 2f - 24, h / 2f, w / 2f + 24, h / 2f, borderPaint);
        canvas.drawLine(w / 2f, h / 2f - 24, w / 2f, h / 2f + 24, borderPaint);

        if (isTouching && touchX >= 0 && touchY >= 0) {
            if (isDragging) {
                canvas.drawCircle(touchX, touchY, 70, touchPaint);
                canvas.drawCircle(touchX, touchY, 75, dragPaint);
                canvas.drawText("SELECTING / DRAGGING", w / 2f, 70, textPaint);
            } else {
                canvas.drawCircle(touchX, touchY, 54, touchPaint);
                canvas.drawCircle(touchX, touchY, 54, touchRingPaint);
            }
        }
    }

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        int pointerCount = event.getPointerCount();
        int action = event.getActionMasked();
        long now = System.currentTimeMillis();

        switch (action) {
            case MotionEvent.ACTION_DOWN: {
                float x = event.getX();
                float y = event.getY();
                touchDownX = x;
                touchDownY = y;
                lastX = x;
                lastY = y;
                touchX = x;
                touchY = y;
                isTouching = true;
                touchDownTime = now;
                isMoving = false;
                isMultiTouch = false;

                long timeSinceLastTap = now - lastTapUpTime;
                float distFromLastTap = (float) Math.hypot(x - lastTapUpX, y - lastTapUpY);
                if (timeSinceLastTap < 300 && distFromLastTap < 100) {
                    isDragging = true;
                    lastTapUpTime = 0;
                    NativeCore.sendMouseClick(1, 0);
                    performHapticFeedback(HapticFeedbackConstants.LONG_PRESS);
                } else {
                    isDragging = false;
                }

                invalidate();
                return true;
            }

            case MotionEvent.ACTION_POINTER_DOWN: {
                if (pointerCount == 2) {
                    isMultiTouch = true;
                    lastScrollX = (event.getX(0) + event.getX(1)) / 2.0f;
                    lastScrollY = (event.getY(0) + event.getY(1)) / 2.0f;
                    twoFingerDownTime = now;
                    hasScrolled = false;

                    if (isDragging) {
                        NativeCore.sendMouseClick(1, 1);
                        isDragging = false;
                    }
                }
                return true;
            }

            case MotionEvent.ACTION_MOVE: {
                if (isMultiTouch && pointerCount >= 2) {
                    float midX = (event.getX(0) + event.getX(1)) / 2.0f;
                    float midY = (event.getY(0) + event.getY(1)) / 2.0f;
                    float dy = midY - lastScrollY;
                    float dx = midX - lastScrollX;

                    if (Math.abs(dy) > 2.5f || Math.abs(dx) > 2.5f) {
                        hasScrolled = true;
                        int scrollStepY = (int) (-dy * 16.0f);
                        int scrollStepX = (int) (dx * 16.0f);
                        NativeCore.sendMouseScroll(scrollStepY, scrollStepX);
                        lastScrollY = midY;
                        lastScrollX = midX;
                    }
                } else if (pointerCount == 1) {
                    float curX = event.getX();
                    float curY = event.getY();
                    float rawDx = (curX - lastX);
                    float rawDy = (curY - lastY);
                    float totalDist = (float) Math.hypot(curX - touchDownX, curY - touchDownY);

                    if (totalDist > 7.0f) {
                        isMoving = true;
                    }

                    float speed = (float) Math.hypot(rawDx, rawDy);
                    float accel = 1.0f + Math.min(speed / 28.0f, 1.8f);
                    float dx = rawDx * sensitivity * accel;
                    float dy = rawDy * sensitivity * accel;

                    if (Math.abs(dx) >= 0.4f || Math.abs(dy) >= 0.4f) {
                        NativeCore.sendMouseMove((int) dx, (int) dy);
                    }

                    lastX = curX;
                    lastY = curY;
                    touchX = curX;
                    touchY = curY;
                    invalidate();
                }
                return true;
            }

            case MotionEvent.ACTION_POINTER_UP: {
                if (isMultiTouch && pointerCount == 2) {
                    long duration = now - twoFingerDownTime;
                    if (!hasScrolled && duration < 320) {
                        performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY);
                        NativeCore.sendMouseClick(2, 2);
                    }
                }
                return true;
            }

            case MotionEvent.ACTION_UP:
            case MotionEvent.ACTION_CANCEL: {
                isTouching = false;
                invalidate();
                long duration = now - touchDownTime;

                if (isDragging) {
                    NativeCore.sendMouseClick(1, 1);
                    isDragging = false;
                    lastTapUpTime = 0;
                    performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY);
                } else if (!isMoving && !isMultiTouch && duration < 240) {
                    performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY);
                    NativeCore.sendMouseClick(1, 2);

                    lastTapUpTime = now;
                    lastTapUpX = event.getX();
                    lastTapUpY = event.getY();
                } else {
                    lastTapUpTime = 0;
                }
                return true;
            }
        }

        return super.onTouchEvent(event);
    }
}
