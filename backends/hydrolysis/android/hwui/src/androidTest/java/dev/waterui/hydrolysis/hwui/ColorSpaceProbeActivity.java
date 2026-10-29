package dev.waterui.hydrolysis.hwui;

import android.app.Activity;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.ColorMatrix;
import android.graphics.ColorMatrixColorFilter;
import android.graphics.LinearGradient;
import android.graphics.Paint;
import android.graphics.RenderEffect;
import android.graphics.RenderNode;
import android.graphics.RuntimeShader;
import android.graphics.Shader;
import android.os.Build;
import android.os.Bundle;
import android.view.View;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;

public final class ColorSpaceProbeActivity extends Activity {
  @Override
  public void onCreate(Bundle state) {
    super.onCreate(state);
    setContentView(new ProbeView(this));
  }
}

final class ProbeView extends View {
  private RenderNode encodedNode;
  private RenderNode linearNode;
  private RenderNode grayBaseNode;
  private RenderNode grayRtNode;
  private RenderNode colorBaseNode;
  private RenderNode colorRtNode;
  private RenderNode checkerNode;
  private final Paint labelPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
  private final Paint swatchPaint = new Paint();
  private final float density;

  ProbeView(android.content.Context context) {
    super(context);
    density = context.getResources().getDisplayMetrics().density;
    labelPaint.setColor(Color.BLACK);
    labelPaint.setTextSize(16f * context.getResources().getDisplayMetrics().scaledDensity);
    setOnApplyWindowInsetsListener((view, insets) -> {
      android.graphics.Insets bars =
          insets.getInsets(android.view.WindowInsets.Type.systemBars());
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom);
      return insets;
    });
  }

  private String readAsset(String name) {
    try {
      InputStream in = getContext().getAssets().open(name);
      try {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buf = new byte[8192];
        int n;
        while ((n = in.read(buf)) >= 0) out.write(buf, 0, n);
        return out.toString(StandardCharsets.UTF_8.name());
      } finally {
        in.close();
      }
    } catch (IOException e) {
      throw new IllegalStateException("missing probe asset " + name, e);
    }
  }

  private RenderNode graySwatchNode(int w, int h, RenderEffect effect) {
    RenderNode node = new RenderNode("swatch");
    node.setPosition(0, 0, w, h);
    Canvas c = node.beginRecording(w, h);
    swatchPaint.setColor(Color.rgb(128, 128, 128));
    c.drawRect(0, 0, w, h, swatchPaint);
    node.endRecording();
    node.setRenderEffect(effect);
    return node;
  }

  private RenderNode gradientNode(int w, int h, boolean translucent, RenderEffect effect) {
    RenderNode node = new RenderNode("gradient");
    node.setPosition(0, 0, w, h);
    Canvas c = node.beginRecording(w, h);
    Paint p = new Paint();
    if (translucent) {
      p.setShader(new LinearGradient(0, 0, w, 0,
          new int[]{0x80FF0000, 0x8000FF00, 0x800000FF},
          null, Shader.TileMode.CLAMP));
    } else {
      p.setShader(new LinearGradient(0, 0, w, 0,
          Color.BLACK, Color.WHITE, Shader.TileMode.CLAMP));
    }
    c.drawRect(0, 0, w, h, p);
    p.setShader(null);
    node.endRecording();
    node.setRenderEffect(effect);
    return node;
  }

  private void checker(Canvas c, int w, int h) {
    Paint p = new Paint();
    int cell = (int)(8 * density);
    p.setColor(0xFFC0C0C0);
    for (int y = 0; y < h; y += cell)
      for (int x = (y / cell % 2) * cell; x < w; x += 2 * cell)
        c.drawRect(x, y, Math.min(x + cell, w), Math.min(y + cell, h), p);
  }

  @Override
  protected void onSizeChanged(int w, int h, int oldw, int oldh) {
    if (Build.VERSION.SDK_INT < 33) {
      encodedNode = null;
      return;
    }
    int margin = (int)(16 * density);
    int swatchW = w - margin * 2;
    int swatchH = (int)(56 * density);
    int gradH = (int)(72 * density);

    ColorMatrix matrix = new ColorMatrix(new float[] {
        .5f,0,0,0,0, 0,.5f,0,0,0, 0,0,.5f,0,0, 0,0,0,1,0
    });
    encodedNode = graySwatchNode(swatchW, swatchH,
        RenderEffect.createColorFilterEffect(new ColorMatrixColorFilter(matrix)));
    linearNode = graySwatchNode(swatchW, swatchH,
        RenderEffect.createRuntimeShaderEffect(
            new RuntimeShader(readAsset("linear_half.agsl")), "inputImage"));

    RuntimeShader rt = new RuntimeShader(readAsset("round_trip.agsl"));
    RenderEffect rtEffect = RenderEffect.createRuntimeShaderEffect(rt, "inputImage");
    grayBaseNode = gradientNode(swatchW, gradH, false, null);
    grayRtNode = gradientNode(swatchW, gradH, false, rtEffect);
    colorBaseNode = gradientNode(swatchW, gradH, true, null);
    colorRtNode = gradientNode(swatchW, gradH, true, rtEffect);
    checkerNode = new RenderNode("checker");
    checkerNode.setPosition(0, 0, swatchW, gradH);
    Canvas cc = checkerNode.beginRecording(swatchW, gradH);
    cc.drawColor(Color.WHITE);
    checker(cc, swatchW, gradH);
    checkerNode.endRecording();
  }

  @Override
  protected void onDraw(Canvas canvas) {
    canvas.drawColor(Color.WHITE);
    float margin = 16 * density;
    float y = getPaddingTop() + margin + 24 * density;
    canvas.drawText("HWUI color-space probe", margin, y, labelPaint);
    y += 20 * density;
    canvas.drawText("Pixel native RenderEffect / API " + Build.VERSION.SDK_INT,
        margin, y, labelPaint);
    y += 30 * density;

    if (Build.VERSION.SDK_INT < 33 || encodedNode == null) {
      canvas.drawText("RuntimeShader requires API 33; device is API "
          + Build.VERSION.SDK_INT, margin, y, labelPaint);
      return;
    }

    float swatchH = 56 * density;
    float gradH = 72 * density;
    float rowGap = 26 * density;

    y += 20 * density;
    canvas.drawText("Native matrix x0.5", margin, y, labelPaint);
    y += 8 * density;
    canvas.save();
    canvas.translate(margin, y);
    canvas.drawRenderNode(encodedNode);
    canvas.restore();
    y += swatchH + 6 * density;
    swatchPaint.setColor(Color.rgb(64, 64, 64));
    canvas.drawRect(margin, y, margin + 12 * density, y + 12 * density, swatchPaint);
    canvas.drawText("Reference RGB 64",
        margin + 18 * density, y + 12 * density, labelPaint);
    y += 16 * density + rowGap;

    canvas.drawText("AGSL linear light x0.5", margin, y, labelPaint);
    y += 8 * density;
    canvas.save();
    canvas.translate(margin, y);
    canvas.drawRenderNode(linearNode);
    canvas.restore();
    y += swatchH + 6 * density;
    swatchPaint.setColor(Color.rgb(92, 92, 92));
    canvas.drawRect(margin, y, margin + 12 * density, y + 12 * density, swatchPaint);
    canvas.drawText("Reference RGB 92",
        margin + 18 * density, y + 12 * density, labelPaint);
    y += 16 * density + rowGap;

    String[] rows = {
        "Grayscale baseline",
        "Grayscale AGSL round trip",
        "Translucent color baseline",
        "Translucent color AGSL round trip",
    };
    RenderNode[] nodes = {grayBaseNode, grayRtNode, colorBaseNode, colorRtNode};
    for (int i = 0; i < 4; i++) {
      canvas.drawText(rows[i], margin, y, labelPaint);
      y += 8 * density;
      canvas.save();
      canvas.translate(margin, y);
      if (i >= 2) canvas.drawRenderNode(checkerNode);
      canvas.drawRenderNode(nodes[i]);
      canvas.restore();
      y += gradH + rowGap;
    }
  }
}
