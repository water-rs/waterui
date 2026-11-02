HydrolysisHostView(context, session).apply {
    {%- if ctx.hydrolysis_android_has_painter_band() %}
    // The painter band is the bottom-most child; platform-view overlays
    // and native embeddings draw above it.
    addView({{ ctx.hydrolysis_android_painter_band_class() }}(context, session), 0)
    {%- endif %}
    {%- if ctx.hydrolysis_android_has_top_band() %}
    // The top band is the last child, above the platform-view overlay: the
    // entries a frame paints over its embedded views present into it.
    addView({{ ctx.hydrolysis_android_top_band_class() }}(context, session))
    {%- endif %}
}