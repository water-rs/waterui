HydrolysisHostView(context, session).apply {
    {%- if ctx.hydrolysis_android_has_painter_band() %}
    // The painter band is the bottom-most child; platform-view overlays
    // and native embeddings draw above it.
    addView({{ ctx.hydrolysis_android_painter_band_class() }}(context, session), 0)
    {%- endif %}
    {%- if ctx.hydrolysis_android_has_region_band() %}
    // The self-drawn content above each embedded platform view draws in
    // the region band the registry stacks right after that view.
    platformViewRegistry.bandFactory = { view ->
        {{ ctx.hydrolysis_android_region_band_class() }}(context, session, view)
    }
    {%- endif %}
}