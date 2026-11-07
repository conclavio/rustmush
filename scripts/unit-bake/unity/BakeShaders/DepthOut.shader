Shader "Hidden/DepthOut" {
  SubShader { Pass { Cull Back ZWrite On
    CGPROGRAM
    #pragma vertex v
    #pragma fragment f
    #include "UnityCG.cginc"
    struct a { float4 p : POSITION; };
    struct o { float4 p : SV_POSITION; float d : TEXCOORD0; };
    // Eye depth, metres, over 150..250 (the camera stands 200 back from the
    // unit), as sixteen bits: the high byte red, the low byte green.
    o v(a i) { o r; r.p = UnityObjectToClipPos(i.p); r.d = -UnityObjectToViewPos(i.p).z; return r; }
    float4 f(o i) : SV_Target {
      float d = saturate((i.d - 150.0) / 100.0);
      float hi = floor(d * 255.0) / 255.0;
      float lo = frac(d * 255.0);
      return float4(hi, lo, 0, 1);
    }
    ENDCG } }
}
