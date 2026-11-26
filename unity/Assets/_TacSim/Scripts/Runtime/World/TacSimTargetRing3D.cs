using TacSim.Core;
using UnityEngine;

namespace TacSim.World
{
    public enum TargetRingAffinity
    {
        Interactable = 0,
        Npc = 1,
        RemotePilot = 2
    }

    /// <summary>
    /// WoW / EverQuest-style glowing 3D ground selection ring placed beneath the targeted
    /// pilot, station/ship NPC, or interactable terminal during 3D walk stages.
    /// </summary>
    public sealed class TacSimTargetRing3D : MonoBehaviour
    {
        private const int RingSegments = 36;
        private const float GroundLiftM = 0.06f;

        private Transform _ringRoot;
        private LineRenderer _outerRing;
        private LineRenderer _innerRing;
        private Material _ringMaterial;
        private Transform _targetTransform;
        private float _baseRadiusM = 0.72f;
        private Color _activeColor = new Color(0.28f, 0.98f, 0.62f, 0.92f);

        public bool IsRingVisible => _ringRoot != null && _ringRoot.gameObject.activeSelf;
        public Transform CurrentTargetTransform => _targetTransform;
        public Color RingColor => _activeColor;
        public float OuterRadiusM => _baseRadiusM;

        public void EnsureVisuals()
        {
            if (_ringRoot != null) return;

            var rootGo = new GameObject("TargetSelectionRingRoot");
            rootGo.transform.SetParent(transform, false);
            _ringRoot = rootGo.transform;

            _ringMaterial = UrpMaterialFactory.GetHologramUnlit("TargetSelectionRingMat", _activeColor);

            var outerGo = new GameObject("OuterRing");
            outerGo.transform.SetParent(_ringRoot, false);
            _outerRing = outerGo.AddComponent<LineRenderer>();
            _outerRing.useWorldSpace = false;
            _outerRing.loop = true;
            _outerRing.startWidth = 0.065f;
            _outerRing.endWidth = 0.065f;
            _outerRing.sharedMaterial = _ringMaterial;
            _outerRing.positionCount = RingSegments;

            var innerGo = new GameObject("InnerRing");
            innerGo.transform.SetParent(_ringRoot, false);
            _innerRing = innerGo.AddComponent<LineRenderer>();
            _innerRing.useWorldSpace = false;
            _innerRing.loop = true;
            _innerRing.startWidth = 0.032f;
            _innerRing.endWidth = 0.032f;
            _innerRing.sharedMaterial = _ringMaterial;
            _innerRing.positionCount = RingSegments;

            PopulateCircleGeometry(_outerRing, _baseRadiusM);
            PopulateCircleGeometry(_innerRing, _baseRadiusM * 0.76f);
            _ringRoot.gameObject.SetActive(false);
        }

        public static Color ColorForAffinity(TargetRingAffinity affinity)
        {
            switch (affinity)
            {
                case TargetRingAffinity.RemotePilot:
                    return new Color(0.26f, 0.92f, 1.00f, 0.92f);
                case TargetRingAffinity.Npc:
                    return new Color(0.28f, 0.98f, 0.62f, 0.92f);
                default:
                    return new Color(1.00f, 0.80f, 0.28f, 0.88f);
            }
        }

        public static TargetRingAffinity ResolveAffinity(WorldInteractable wi)
        {
            if (wi == null) return TargetRingAffinity.Interactable;
            if (wi.kind == InteractableKind.NpcActor)
                return wi.isRemotePlayer ? TargetRingAffinity.RemotePilot : TargetRingAffinity.Npc;
            return TargetRingAffinity.Interactable;
        }

        public void SetTarget(WorldInteractable wi)
        {
            EnsureVisuals();
            if (wi == null || !wi.isActiveAndEnabled)
            {
                ClearTarget();
                return;
            }

            _targetTransform = wi.transform;
            var affinity = ResolveAffinity(wi);
            _activeColor = ColorForAffinity(affinity);
            _baseRadiusM = wi.kind == InteractableKind.NpcActor ? 0.68f : 0.95f;

            ApplyVisualStyle();
            SyncRingPosition();
            _ringRoot.gameObject.SetActive(true);
        }

        public void ClearTarget()
        {
            _targetTransform = null;
            if (_ringRoot != null && _ringRoot.gameObject.activeSelf)
                _ringRoot.gameObject.SetActive(false);
        }

        private void ApplyVisualStyle()
        {
            if (_outerRing != null)
            {
                _outerRing.startColor = _activeColor;
                _outerRing.endColor = _activeColor;
                PopulateCircleGeometry(_outerRing, _baseRadiusM);
            }
            if (_innerRing != null)
            {
                Color innerCol = new Color(_activeColor.r, _activeColor.g, _activeColor.b, _activeColor.a * 0.65f);
                _innerRing.startColor = innerCol;
                _innerRing.endColor = innerCol;
                PopulateCircleGeometry(_innerRing, _baseRadiusM * 0.76f);
            }
            if (_ringMaterial != null)
            {
                _ringMaterial.color = _activeColor;
            }
        }

        private void SyncRingPosition()
        {
            if (_ringRoot == null || _targetTransform == null) return;
            Vector3 p = _targetTransform.position;
            _ringRoot.position = new Vector3(p.x, p.y + GroundLiftM, p.z);
        }

        private void LateUpdate()
        {
            if (_ringRoot == null || !_ringRoot.gameObject.activeSelf) return;

            if (_targetTransform == null || !_targetTransform.gameObject.activeInHierarchy)
            {
                ClearTarget();
                return;
            }

            SyncRingPosition();

            float t = Time.unscaledTime;
            float pulse = 1f + Mathf.Sin(t * 4.2f) * 0.055f;
            _ringRoot.localScale = new Vector3(pulse, 1f, pulse);
            if (_innerRing != null)
            {
                _innerRing.transform.localRotation = Quaternion.Euler(0f, t * 48f, 0f);
            }
        }

        private static void PopulateCircleGeometry(LineRenderer lr, float radiusM)
        {
            if (lr == null) return;
            for (int i = 0; i < RingSegments; i++)
            {
                float ang = (i / (float)RingSegments) * Mathf.PI * 2f;
                lr.SetPosition(i, new Vector3(Mathf.Cos(ang) * radiusM, 0f, Mathf.Sin(ang) * radiusM));
            }
        }
    }
}
