HydrolysisHostView(context, session).apply {
    {%- if ctx.hydrolysis_android_has_painter_band() %}
    // The painter band is the bottom-most child; platform-view overlays
    // and native embeddings draw above it.
    addView({{ ctx.hydrolysis_android_painter_band_class() }}(context, session), 0)
    {%- endif %}
}