Shader "Hidden/FlatId" {
  Properties { _Color ("c", Color) = (0,1,0,1) }
  SubShader { Pass { Cull Back ZWrite On
    CGPROGRAM
    #pragma vertex v
    #pragma fragment f
    #include "UnityCG.cginc"
    float4 _Color;
    struct a { float4 p : POSITION; };
    struct o { float4 p : SV_POSITION; };
    o v(a i) { o r; r.p = UnityObjectToClipPos(i.p); return r; }
    float4 f(o i) : SV_Target { return float4(_Color.rgb, 1); }
    ENDCG } }
}
