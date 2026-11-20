using System;
using System.Collections.Generic;
using TacSim.Core;
using UnityEngine;

namespace TacSim.Character
{
    /// <summary>
    /// Instantiates Ida Faber character prefabs (Stellar Trooper / Cyberpunk Street Fighter for player,
    /// Techwear Girls / Stellar Girl Astra / Stellar Girl Celeste for station cast), sanitizes any
    /// vendor hair/shader issues for URP, and drives grounded procedural Humanoid locomotion over
    /// Ida Faber's authored Idle pose so feet stay planted on the deck and hands keep natural posture.
    /// </summary>
    public sealed class CharacterVisualSpawner : MonoBehaviour
    {
        private GameObject _activeModel;
        private Animator _animator;

        // Cached Humanoid bones for LateUpdate grounded stride / helm pose over A_Idle
        private bool _hasHumanoidBones;
        private Transform _hips;
        private Transform _spine;
        private Transform _chest;
        private Transform _leftUpperLeg;
        private Transform _rightUpperLeg;
        private Transform _leftLowerLeg;
        private Transform _rightLowerLeg;
        private Transform _leftFoot;
        private Transform _rightFoot;
        private Transform _leftUpperArm;
        private Transform _rightUpperArm;
        private Transform _leftLowerArm;
        private Transform _rightLowerArm;

        private bool _sampledIdleAnkle;
        private float _idleAnkleHeight = 0.095f;
        private Renderer[] _cachedRenderers;
        private bool _hasAuthoredLocomotionClips;
        private bool _hasAuthoredSitClip;
        private bool _isVisible = true;

        public bool HasAuthoredLocomotionClips => _hasAuthoredLocomotionClips;
        public bool HasActiveVisual => _activeModel != null;

        // Locomotion blend state
        private float _planarSpeed;
        private bool _isSprinting;
        private bool _isSeated;
        private float _locoBlend;
        private float _sprintBlend;
        private float _seatBlend;
        private float _walkCyclePhase;

        // Fallback articulated suit pivots when commercial FBX packs are not imported
        private Transform _leftLegFallback;
        private Transform _rightLegFallback;
        private Transform _leftArmFallback;
        private Transform _rightArmFallback;

        private static readonly int HashSpeed = Animator.StringToHash("Speed");
        private static readonly int HashSprinting = Animator.StringToHash("IsSprinting");
        private static readonly int HashSeated = Animator.StringToHash("Seated");

        private static readonly Dictionary<Material, Material> RepairedMaterialCache = new Dictionary<Material, Material>();

        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.SubsystemRegistration)]
        public static void ClearRepairedMaterialCache()
        {
            foreach (var kv in RepairedMaterialCache)
            {
                if (kv.Value != null && kv.Value != kv.Key)
                {
                    if (Application.isPlaying) Destroy(kv.Value);
                    else DestroyImmediate(kv.Value);
                }
            }
            RepairedMaterialCache.Clear();
        }

        public void SetVisible(bool visible)
        {
            _isVisible = visible;
            if (_activeModel != null && _activeModel.activeSelf != visible)
                _activeModel.SetActive(visible);
        }

        public void BuildPlayerVisual(TacSimAssetCatalog catalog, PlayerAvatarPreset preset)
        {
            ClearCurrent();
            GameObject prefab = catalog != null ? catalog.GetPlayerPrefab(preset) : null;
            Color suitColor = preset == PlayerAvatarPreset.StellarTrooperMale || preset == PlayerAvatarPreset.StellarTrooperFemale
                ? new Color(0.22f, 0.34f, 0.48f)
                : new Color(0.32f, 0.18f, 0.38f);
            Color visorColor = new Color(0.2f, 0.85f, 1.0f) * 1.8f;
            AttachPrefabOrFallback(prefab, catalog != null ? catalog.humanoidAnimator : null, suitColor, visorColor);
        }

        public void BuildNpcVisual(TacSimAssetCatalog catalog, string npcStableKey, bool isRemotePlayer = false, bool isMale = false)
        {
            ClearCurrent();
            GameObject prefab = null;
            if (catalog != null)
            {
                prefab = isRemotePlayer
                    ? catalog.GetPlayerPrefab(isMale ? PlayerAvatarPreset.CyberpunkStreetFighterMale : PlayerAvatarPreset.CyberpunkStreetFighterFemale)
                    : catalog.GetNpcPrefabForKey(npcStableKey, isMale);
            }

            uint h = 2166136261u;
            if (!string.IsNullOrEmpty(npcStableKey))
            {
                for (int i = 0; i < npcStableKey.Length; i++)
                    h = (h ^ npcStableKey[i]) * 16777619u;
            }
            float hue = (h % 360u) / 360f;
            Color suit = Color.HSVToRGB(hue, 0.38f, 0.52f);
            Color accent = isRemotePlayer ? new Color(0.3f, 1f, 0.55f) * 1.5f : new Color(1.0f, 0.72f, 0.25f) * 1.5f;
            AttachPrefabOrFallback(prefab, catalog != null ? catalog.humanoidAnimator : null, suit, accent);
        }

        public void UpdateLocomotion(float planarSpeedMps, bool isSprinting, bool isSeated, float dt)
        {
            float safeDt = Mathf.Max(0.0005f, dt);
            _planarSpeed = planarSpeedMps;
            _isSprinting = isSprinting;
            _isSeated = isSeated;

            float targetLoco = (!isSeated && planarSpeedMps > 0.12f) ? Mathf.Clamp01(planarSpeedMps / 4.2f) : 0f;
            float targetSprint = (!isSeated && isSprinting && planarSpeedMps > 5.0f) ? 1f : 0f;
            float targetSeat = isSeated ? 1f : 0f;

            _locoBlend = Mathf.MoveTowards(_locoBlend, targetLoco, safeDt * 7.5f);
            _sprintBlend = Mathf.MoveTowards(_sprintBlend, targetSprint, safeDt * 6.0f);
            _seatBlend = Mathf.MoveTowards(_seatBlend, targetSeat, safeDt * 6.5f);

            if (!isSeated && planarSpeedMps > 0.10f)
            {
                float cadenceRadPerSec = Mathf.Lerp(6.8f, 11.4f, _sprintBlend) * Mathf.Clamp(planarSpeedMps / 4.6f, 0.55f, 1.35f);
                _walkCyclePhase += safeDt * cadenceRadPerSec;
                if (_walkCyclePhase > Mathf.PI * 200f)
                    _walkCyclePhase -= Mathf.PI * 200f;
            }

            if (_animator != null && _animator.runtimeAnimatorController != null)
            {
                // When authored Walk/Run/Sit mocap clips are bound into TacSimHumanoid.controller, drive Mecanim
                // state transitions directly; otherwise keep Mecanim in authored Idle mocap (`A_Idle_F`) so pelvis
                // height, breathing, neck/head posture, and finger poses stay intact while LateUpdate drives stride!
                _animator.SetFloat(HashSpeed, (_hasAuthoredLocomotionClips && !isSeated) ? planarSpeedMps : 0f);
                _animator.SetBool(HashSprinting, _hasAuthoredLocomotionClips && !isSeated && isSprinting);
                _animator.SetBool(HashSeated, _hasAuthoredSitClip && isSeated);
                return;
            }

            // Articulated procedural stride on the fallback suit when commercial FBX assets are not imported
            if (_leftLegFallback != null && _rightLegFallback != null)
            {
                if (isSeated)
                {
                    _leftLegFallback.localRotation = Quaternion.Euler(-75f, 0f, 0f);
                    _rightLegFallback.localRotation = Quaternion.Euler(-75f, 0f, 0f);
                    if (_leftArmFallback != null) _leftArmFallback.localRotation = Quaternion.Euler(-38f, 0f, 0f);
                    if (_rightArmFallback != null) _rightArmFallback.localRotation = Quaternion.Euler(-38f, 0f, 0f);
                }
                else if (planarSpeedMps > 0.1f)
                {
                    float swing = Mathf.Sin(_walkCyclePhase) * Mathf.Clamp(planarSpeedMps * 7f, 10f, 34f);
                    _leftLegFallback.localRotation = Quaternion.Euler(-swing, 0f, 0f);
                    _rightLegFallback.localRotation = Quaternion.Euler(swing, 0f, 0f);
                    if (_leftArmFallback != null) _leftArmFallback.localRotation = Quaternion.Euler(swing * 0.7f, 0f, 0f);
                    if (_rightArmFallback != null) _rightArmFallback.localRotation = Quaternion.Euler(-swing * 0.7f, 0f, 0f);
                }
                else
                {
                    _leftLegFallback.localRotation = Quaternion.Slerp(_leftLegFallback.localRotation, Quaternion.identity, safeDt * 10f);
                    _rightLegFallback.localRotation = Quaternion.Slerp(_rightLegFallback.localRotation, Quaternion.identity, safeDt * 10f);
                    if (_leftArmFallback != null) _leftArmFallback.localRotation = Quaternion.Slerp(_leftArmFallback.localRotation, Quaternion.identity, safeDt * 10f);
                    if (_rightArmFallback != null) _rightArmFallback.localRotation = Quaternion.Slerp(_rightArmFallback.localRotation, Quaternion.identity, safeDt * 10f);
                }
            }
        }

        private void LateUpdate()
        {
            if (_animator == null)
                return;

            if (!_hasHumanoidBones)
                CacheHumanoidBones();
            if (!_hasHumanoidBones)
                return;

            Vector3 pitchAxis = transform.right;

            // 1. Ensure an upright standing base pose before layering seated or stride rotations:
            // Catches both knee-squat (> 20 deg) and forward/backward hip tilt (> 14 deg from -transform.up)
            // on un-rebuilt clips while leaving subtle mocap idle weight-shifts (~3-6 deg) and authored mocap clips untouched!
            bool allowStandingGuard = (!_hasAuthoredLocomotionClips || _locoBlend < 0.05f) &&
                                      (!_hasAuthoredSitClip || _seatBlend < 0.05f);
            if (allowStandingGuard &&
                _leftUpperLeg != null && _leftLowerLeg != null && _leftFoot != null &&
                _rightUpperLeg != null && _rightLowerLeg != null && _rightFoot != null)
            {
                Vector3 downDir = -transform.up;
                Vector3 lThigh = _leftLowerLeg.position - _leftUpperLeg.position;
                Vector3 lShin = _leftFoot.position - _leftLowerLeg.position;
                Vector3 rThigh = _rightLowerLeg.position - _rightUpperLeg.position;
                Vector3 rShin = _rightFoot.position - _rightLowerLeg.position;

                bool leftOffVertical = lThigh.sqrMagnitude > 0.001f && lShin.sqrMagnitude > 0.001f &&
                                       (Vector3.Angle(lThigh, lShin) > 20f || Vector3.Angle(lThigh, downDir) > 14f);
                bool rightOffVertical = rThigh.sqrMagnitude > 0.001f && rShin.sqrMagnitude > 0.001f &&
                                        (Vector3.Angle(rThigh, rShin) > 20f || Vector3.Angle(rThigh, downDir) > 14f);
                if (leftOffVertical || rightOffVertical)
                {
                    StraightenStandingLeg(_leftUpperLeg, _leftLowerLeg, _leftFoot, pitchAxis);
                    StraightenStandingLeg(_rightUpperLeg, _rightLowerLeg, _rightFoot, pitchAxis);
                }
            }

            // Sample exact boot-sole thickness below the ankle joint from actual rendered bounds on frame 1
            if (!_sampledIdleAnkle && _locoBlend < 0.01f && _seatBlend < 0.01f && _leftFoot != null && _rightFoot != null)
            {
                float lowestFootJointY = Mathf.Min(_leftFoot.position.y, _rightFoot.position.y);
                if (TryGetLowestRenderedY(out float minMeshY))
                {
                    float soleThickness = lowestFootJointY - minMeshY;
                    if (soleThickness >= 0.01f && soleThickness <= 0.25f)
                        _idleAnkleHeight = soleThickness;
                }
                _sampledIdleAnkle = true;
            }

            // 2. Bridge Helm Seated Pose
            if (_seatBlend > 0.001f)
            {
                if (!_hasAuthoredSitClip)
                {
                    float b = _seatBlend;
                    if (_leftUpperLeg != null) _leftUpperLeg.Rotate(pitchAxis, -75f * b, UnityEngine.Space.World);
                    if (_rightUpperLeg != null) _rightUpperLeg.Rotate(pitchAxis, -75f * b, UnityEngine.Space.World);
                    if (_leftLowerLeg != null) _leftLowerLeg.Rotate(pitchAxis, 75f * b, UnityEngine.Space.World);
                    if (_rightLowerLeg != null) _rightLowerLeg.Rotate(pitchAxis, 75f * b, UnityEngine.Space.World);
                    if (_leftUpperArm != null) _leftUpperArm.Rotate(pitchAxis, -32f * b, UnityEngine.Space.World);
                    if (_rightUpperArm != null) _rightUpperArm.Rotate(pitchAxis, -32f * b, UnityEngine.Space.World);
                    if (_leftLowerArm != null) _leftLowerArm.Rotate(pitchAxis, -36f * b, UnityEngine.Space.World);
                    if (_rightLowerArm != null) _rightLowerArm.Rotate(pitchAxis, -36f * b, UnityEngine.Space.World);
                    if (_hips != null) _hips.position += Vector3.down * (0.34f * b);
                }
                return;
            }

            // 3. Grounded Idle & Walking / Sprinting Stride (procedural when only Idle mocap is present)
            if (!_hasAuthoredLocomotionClips && _locoBlend > 0.001f)
            {
                float s = Mathf.Sin(_walkCyclePhase);
                float c = Mathf.Cos(_walkCyclePhase);

                float hipAmp = Mathf.Lerp(22f, 36f, _sprintBlend) * _locoBlend;
                float kneeAmp = Mathf.Lerp(30f, 48f, _sprintBlend) * _locoBlend;
                float armAmp = Mathf.Lerp(18f, 34f, _sprintBlend) * _locoBlend;
                float elbowBase = Mathf.Lerp(10f, 30f, _sprintBlend) * _locoBlend;
                float spineLean = Mathf.Lerp(3.0f, 8.5f, _sprintBlend) * _locoBlend;

                // Thigh stride: negative angle around +right swings a downward leg FORWARD (+Z) in Unity
                if (_leftUpperLeg != null) _leftUpperLeg.Rotate(pitchAxis, -s * hipAmp, UnityEngine.Space.World);
                if (_rightUpperLeg != null) _rightUpperLeg.Rotate(pitchAxis, s * hipAmp, UnityEngine.Space.World);

                // Knee flexion: positive angle bends shin BACKWARD; peaks during forward swing (c > 0 for left, -c > 0 for right)
                float leftKnee = (Mathf.Max(0f, c) * kneeAmp) + (2f * _locoBlend);
                float rightKnee = (Mathf.Max(0f, -c) * kneeAmp) + (2f * _locoBlend);
                if (_leftLowerLeg != null) _leftLowerLeg.Rotate(pitchAxis, leftKnee, UnityEngine.Space.World);
                if (_rightLowerLeg != null) _rightLowerLeg.Rotate(pitchAxis, rightKnee, UnityEngine.Space.World);

                // Subtle toe-lift (dorsiflexion) during forward swing so boots clear the deck
                if (_leftFoot != null) _leftFoot.Rotate(pitchAxis, -Mathf.Max(0f, c) * 10f * _locoBlend, UnityEngine.Space.World);
                if (_rightFoot != null) _rightFoot.Rotate(pitchAxis, -Mathf.Max(0f, -c) * 10f * _locoBlend, UnityEngine.Space.World);

                // Forward torso lean + natural counter-twist
                if (_spine != null) _spine.Rotate(pitchAxis, spineLean, UnityEngine.Space.World);
                if (_chest != null) _chest.Rotate(Vector3.up, s * 4.5f * _locoBlend, UnityEngine.Space.World);

                // Counter-swinging arms & natural elbow flexion
                if (_leftUpperArm != null) _leftUpperArm.Rotate(pitchAxis, s * armAmp, UnityEngine.Space.World);
                if (_rightUpperArm != null) _rightUpperArm.Rotate(pitchAxis, -s * armAmp, UnityEngine.Space.World);
                if (_leftLowerArm != null) _leftLowerArm.Rotate(pitchAxis, -(elbowBase + Mathf.Max(0f, -s) * 10f * _locoBlend), UnityEngine.Space.World);
                if (_rightLowerArm != null) _rightLowerArm.Rotate(pitchAxis, -(elbowBase + Mathf.Max(0f, s) * 10f * _locoBlend), UnityEngine.Space.World);
            }

            // Plant lowest boot sole cleanly on the floor (`y = transform.position.y`) both at idle and during stride!
            if (_hips != null && _leftFoot != null && _rightFoot != null)
            {
                float lowestSoleY = Mathf.Min(_leftFoot.position.y, _rightFoot.position.y) - _idleAnkleHeight;
                float flightHop = _locoBlend > 0.01f ? (_sprintBlend * Mathf.Abs(Mathf.Sin(_walkCyclePhase)) * 0.022f) : 0f;
                float deltaY = (transform.position.y + flightHop) - lowestSoleY;
                _hips.position += Vector3.up * Mathf.Clamp(deltaY, -0.35f, 0.35f);
            }
        }

        private void StraightenStandingLeg(Transform upperLeg, Transform lowerLeg, Transform foot, Vector3 pitchAxis)
        {
            Vector3 downDir = -transform.up;
            Vector3 targetThigh = Quaternion.AngleAxis(-1.6f, pitchAxis) * downDir;
            Vector3 targetShin = Quaternion.AngleAxis(1.6f, pitchAxis) * downDir;

            Vector3 curThigh = (lowerLeg.position - upperLeg.position).normalized;
            if (curThigh.sqrMagnitude > 0.001f)
                upperLeg.rotation = Quaternion.FromToRotation(curThigh, targetThigh) * upperLeg.rotation;

            Vector3 curShin = (foot.position - lowerLeg.position).normalized;
            if (curShin.sqrMagnitude > 0.001f)
                lowerLeg.rotation = Quaternion.FromToRotation(curShin, targetShin) * lowerLeg.rotation;
        }

        private bool TryGetLowestRenderedY(out float minY)
        {
            minY = float.MaxValue;
            if (_activeModel == null) return false;
            if (_cachedRenderers == null || _cachedRenderers.Length == 0)
                _cachedRenderers = _activeModel.GetComponentsInChildren<Renderer>(false);
            bool any = false;
            for (int i = 0; i < _cachedRenderers.Length; i++)
            {
                var r = _cachedRenderers[i];
                if (r == null || !r.enabled || !r.gameObject.activeInHierarchy) continue;
                Bounds b = r.bounds;
                if (b.size.sqrMagnitude < 0.0001f) continue;
                if (b.min.y < minY) minY = b.min.y;
                any = true;
            }
            return any;
        }

        private void ClearCurrent()
        {
            _animator = null;
            _hasHumanoidBones = false;
            _hasAuthoredLocomotionClips = false;
            _hasAuthoredSitClip = false;
            _sampledIdleAnkle = false;
            _cachedRenderers = null;
            _hips = _spine = _chest = null;
            _leftUpperLeg = _rightUpperLeg = _leftLowerLeg = _rightLowerLeg = _leftFoot = _rightFoot = null;
            _leftUpperArm = _rightUpperArm = _leftLowerArm = _rightLowerArm = null;
            _leftLegFallback = _rightLegFallback = _leftArmFallback = _rightArmFallback = null;
            _activeModel = null;

            // Destroy ALL visual children (including any pre-existing scene FallbackSuitRig or CharacterModel),
            // preserving only NPC Billboard nameplates.
            for (int i = transform.childCount - 1; i >= 0; i--)
            {
                Transform child = transform.GetChild(i);
                if (child == null || child.name == "Billboard")
                    continue;

                if (Application.isPlaying)
                {
                    child.gameObject.SetActive(false);
                    Destroy(child.gameObject);
                }
                else
                {
                    DestroyImmediate(child.gameObject);
                }
            }
        }

        private void AttachPrefabOrFallback(GameObject prefab, RuntimeAnimatorController controller, Color suitColor, Color visorHdr)
        {
            if (prefab != null)
            {
                _activeModel = Instantiate(prefab, transform);
                _activeModel.name = "CharacterModel";
                _activeModel.transform.localPosition = Vector3.zero;
                _activeModel.transform.localRotation = Quaternion.identity;
                _activeModel.transform.localScale = Vector3.one;

                // Strip any stray colliders on imported character prefabs so CharacterController stays clean
                foreach (var col in _activeModel.GetComponentsInChildren<Collider>(true))
                {
                    col.enabled = false;
                }

                SanitizeCharacterMaterials(_activeModel);

                _animator = _activeModel.GetComponentInChildren<Animator>();
                if (_animator == null)
                    _animator = _activeModel.AddComponent<Animator>();

                if (controller != null)
                {
                    _animator.runtimeAnimatorController = controller;
                    DetectAuthoredMocapClips(controller);
                }
                _animator.applyRootMotion = false;
                _animator.cullingMode = AnimatorCullingMode.AlwaysAnimate;

                CacheHumanoidBones();
                if (!_isVisible && _activeModel != null)
                    _activeModel.SetActive(false);
                return;
            }

            // Articulated URP flight-suit mannequin fallback before commercial packs are imported
            _activeModel = new GameObject("FallbackSuitRig");
            _activeModel.transform.SetParent(transform, false);
            if (!_isVisible)
                _activeModel.SetActive(false);

            Material suitMat = UrpMaterialFactory.GetOpaque($"Suit_{suitColor}", suitColor, 0.48f, 0.35f);
            Material darkMat = UrpMaterialFactory.GetOpaque("SuitJoints", new Color(0.12f, 0.14f, 0.17f), 0.3f, 0.5f);
            Material visorMat = UrpMaterialFactory.GetEmissive($"Visor_{visorHdr}", new Color(0.1f, 0.25f, 0.35f), visorHdr);

            CreatePart(_activeModel.transform, "Torso", PrimitiveType.Capsule, new Vector3(0f, 1.12f, 0f), new Vector3(0.42f, 0.36f, 0.26f), suitMat);
            CreatePart(_activeModel.transform, "Backpack", PrimitiveType.Cube, new Vector3(0f, 1.16f, -0.17f), new Vector3(0.28f, 0.36f, 0.12f), darkMat);
            CreatePart(_activeModel.transform, "Helmet", PrimitiveType.Sphere, new Vector3(0f, 1.62f, 0f), new Vector3(0.26f, 0.28f, 0.27f), suitMat);
            CreatePart(_activeModel.transform, "Visor", PrimitiveType.Cube, new Vector3(0f, 1.63f, 0.11f), new Vector3(0.19f, 0.08f, 0.08f), visorMat);

            _leftLegFallback = CreateLimbPivot(_activeModel.transform, "LeftLeg", new Vector3(-0.11f, 0.82f, 0f), new Vector3(0f, -0.40f, 0f), new Vector3(0.14f, 0.40f, 0.14f), darkMat);
            _rightLegFallback = CreateLimbPivot(_activeModel.transform, "RightLeg", new Vector3(0.11f, 0.82f, 0f), new Vector3(0f, -0.40f, 0f), new Vector3(0.14f, 0.40f, 0.14f), darkMat);
            _leftArmFallback = CreateLimbPivot(_activeModel.transform, "LeftArm", new Vector3(-0.26f, 1.36f, 0f), new Vector3(0f, -0.28f, 0f), new Vector3(0.11f, 0.28f, 0.11f), suitMat);
            _rightArmFallback = CreateLimbPivot(_activeModel.transform, "RightArm", new Vector3(0.26f, 1.36f, 0f), new Vector3(0f, -0.28f, 0f), new Vector3(0.11f, 0.28f, 0.11f), suitMat);
        }

        private void DetectAuthoredMocapClips(RuntimeAnimatorController controller)
        {
            _hasAuthoredLocomotionClips = false;
            _hasAuthoredSitClip = false;
            if (controller == null) return;

            AnimationClip[] clips = controller.animationClips;
            if (clips == null) return;

            for (int i = 0; i < clips.Length; i++)
            {
                var c = clips[i];
                if (c == null || string.IsNullOrEmpty(c.name))
                    continue;
                if (c.name.StartsWith("TacSim_", StringComparison.OrdinalIgnoreCase))
                    continue;

                if (c.name.IndexOf("Walk", StringComparison.OrdinalIgnoreCase) >= 0)
                    _hasAuthoredLocomotionClips = true;
                if (c.name.IndexOf("Sit", StringComparison.OrdinalIgnoreCase) >= 0 ||
                    c.name.IndexOf("Seat", StringComparison.OrdinalIgnoreCase) >= 0)
                    _hasAuthoredSitClip = true;
            }
        }

        private void CacheHumanoidBones()
        {
            if (_animator == null || !_animator.isHuman)
            {
                _hasHumanoidBones = false;
                return;
            }

            _hips = _animator.GetBoneTransform(HumanBodyBones.Hips);
            _spine = _animator.GetBoneTransform(HumanBodyBones.Spine);
            _chest = _animator.GetBoneTransform(HumanBodyBones.Chest);
            _leftUpperLeg = _animator.GetBoneTransform(HumanBodyBones.LeftUpperLeg);
            _rightUpperLeg = _animator.GetBoneTransform(HumanBodyBones.RightUpperLeg);
            _leftLowerLeg = _animator.GetBoneTransform(HumanBodyBones.LeftLowerLeg);
            _rightLowerLeg = _animator.GetBoneTransform(HumanBodyBones.RightLowerLeg);
            _leftFoot = _animator.GetBoneTransform(HumanBodyBones.LeftFoot);
            _rightFoot = _animator.GetBoneTransform(HumanBodyBones.RightFoot);
            _leftUpperArm = _animator.GetBoneTransform(HumanBodyBones.LeftUpperArm);
            _rightUpperArm = _animator.GetBoneTransform(HumanBodyBones.RightUpperArm);
            _leftLowerArm = _animator.GetBoneTransform(HumanBodyBones.LeftLowerArm);
            _rightLowerArm = _animator.GetBoneTransform(HumanBodyBones.RightLowerArm);

            _hasHumanoidBones = _hips != null && _leftUpperLeg != null && _rightUpperLeg != null;
        }

        private static void SanitizeCharacterMaterials(GameObject root)
        {
            Renderer[] renderers = root.GetComponentsInChildren<Renderer>(true);

            // Detect if the character already has an active primary 3D hair mesh (e.g., SK_TROOPER_M_HAIR_07)
            bool hasPrimaryHairMesh = false;
            foreach (var r in renderers)
            {
                if (r == null || !r.enabled || !r.gameObject.activeInHierarchy) continue;
                string rn = r.name.ToUpperInvariant();
                if (rn.Contains("HAIR") && !rn.Contains("BUZZCUT") && !rn.Contains("HAIRHAT"))
                {
                    hasPrimaryHairMesh = true;
                    break;
                }
            }

            foreach (var r in renderers)
            {
                if (r == null) continue;
                string rUpper = r.name.ToUpperInvariant();

                // Hide redundant buzzcut scalp cap when a full 3D hairstyle mesh is already active on the head
                if (hasPrimaryHairMesh && (rUpper.Contains("BUZZCUT") || rUpper.Contains("HAIRHAT")))
                {
                    r.enabled = false;
                    continue;
                }

                Material[] mats = r.sharedMaterials;
                if (mats == null || mats.Length == 0) continue;

                bool changed = false;
                var cleaned = new List<Material>(mats.Length);

                for (int i = 0; i < mats.Length; i++)
                {
                    Material m = mats[i];
                    if (m == null)
                    {
                        // Null material slots in multi-pass prefabs render bright magenta in URP; strip them
                        changed = true;
                        continue;
                    }

                    Material repaired = GetSanitizedMaterialOrSelf(m);
                    if (repaired != m)
                        changed = true;
                    cleaned.Add(repaired);
                }

                if (cleaned.Count == 0)
                {
                    cleaned.Add(UrpMaterialFactory.GetOpaque("CharFallbackDark", new Color(0.10f, 0.10f, 0.11f), 0.25f, 0.1f));
                    changed = true;
                }

                if (changed)
                    r.sharedMaterials = cleaned.ToArray();
            }
        }

        private static Material GetSanitizedMaterialOrSelf(Material src)
        {
            if (RepairedMaterialCache.TryGetValue(src, out Material cached) && cached != null)
                return cached;

            Shader sh = src.shader;
            string shName = sh != null ? sh.name : string.Empty;
            bool brokenShader = sh == null ||
                                !sh.isSupported ||
                                shName == "Hidden/InternalErrorShader" ||
                                shName == "Standard" ||
                                shName == "Standard (Specular setup)" ||
                                shName.StartsWith("Legacy Shaders/", StringComparison.Ordinal);

            if (brokenShader)
            {
                Shader urpLit = Shader.Find("Universal Render Pipeline/Lit");
                if (urpLit == null) return src;

                Texture mainTex = TryGetFirstTexture(src, "_BaseMap", "_Basecolor_Map", "_BaseColor_Map", "_BaseColorMap", "_MainTex");
                Color tint = src.HasProperty("_BaseColor") ? src.GetColor("_BaseColor")
                           : (src.HasProperty("_Color") ? src.GetColor("_Color") : new Color(0.12f, 0.11f, 0.11f));

                var fallback = new Material(urpLit) { name = src.name + "_URP" };
                if (mainTex != null && fallback.HasProperty("_BaseMap"))
                    fallback.SetTexture("_BaseMap", mainTex);
                if (fallback.HasProperty("_BaseColor"))
                    fallback.SetColor("_BaseColor", tint);
                if (fallback.HasProperty("_Smoothness"))
                    fallback.SetFloat("_Smoothness", 0.25f);

                RepairedMaterialCache[src] = fallback;
                return fallback;
            }

            // Repair Ida Faber hair/hairhat materials whose _Basecolor_Map points to Placeholder_01 or lacks _ALPHATEST_ON
            bool isHairMat = shName.IndexOf("Hair", StringComparison.OrdinalIgnoreCase) >= 0 ||
                             src.name.IndexOf("HAIR", StringComparison.OrdinalIgnoreCase) >= 0;
            if (isHairMat && src.HasProperty("_Basecolor_Map"))
            {
                Texture bc = src.GetTexture("_Basecolor_Map");
                bool isPlaceholder = bc == null || bc.name.IndexOf("Placeholder", StringComparison.OrdinalIgnoreCase) >= 0;
                bool missingAlphaKeyword = !src.IsKeywordEnabled("_ALPHATEST_ON");

                if (isPlaceholder || missingAlphaKeyword)
                {
                    var clone = new Material(src) { name = src.name + "_Fixed" };
                    clone.EnableKeyword("_ALPHATEST_ON");
                    if (clone.HasProperty("_AlphaClip")) clone.SetFloat("_AlphaClip", 1f);
                    if (clone.HasProperty("_AlphaCutoffEnable")) clone.SetFloat("_AlphaCutoffEnable", 1f);
                    if (isPlaceholder && clone.HasProperty("_Root_Color"))
                        clone.SetColor("_Root_Color", new Color(0.11f, 0.09f, 0.08f, 1f));

                    RepairedMaterialCache[src] = clone;
                    return clone;
                }
            }

            return src;
        }

        private static Texture TryGetFirstTexture(Material mat, params string[] propNames)
        {
            foreach (string p in propNames)
            {
                if (mat.HasProperty(p))
                {
                    Texture t = mat.GetTexture(p);
                    if (t != null && t.name.IndexOf("Placeholder", StringComparison.OrdinalIgnoreCase) < 0)
                        return t;
                }
            }
            return null;
        }

        private static Transform CreateLimbPivot(Transform parent, string name, Vector3 hipPos, Vector3 meshOffset, Vector3 scale, Material mat)
        {
            var pivot = new GameObject(name).transform;
            pivot.SetParent(parent, false);
            pivot.localPosition = hipPos;
            CreatePart(pivot, name + "_Mesh", PrimitiveType.Capsule, meshOffset, scale, mat);
            return pivot;
        }

        private static void CreatePart(Transform parent, string name, PrimitiveType type, Vector3 localPos, Vector3 localScale, Material mat)
        {
            var go = GameObject.CreatePrimitive(type);
            go.name = name;
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            go.transform.localScale = localScale;
            var col = go.GetComponent<Collider>();
            if (col != null)
            {
                if (Application.isPlaying) Destroy(col);
                else DestroyImmediate(col);
            }
            var mr = go.GetComponent<MeshRenderer>();
            if (mr != null) mr.sharedMaterial = mat;
        }
    }
}
