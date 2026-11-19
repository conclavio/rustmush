using System;
using TacSim.Core;
using TMPro;
using UnityEngine;

namespace TacSim.World
{
    public sealed class SpawnedGroundVehicleRig
    {
        public GameObject Root;
        public Transform HullTiltRoot;
        public Transform ChassisTransform;
        public Transform WaistMountSocket;
        public Transform TurretPivot;
        public Transform MuzzlePoint;
        public Transform CupolaCameraAnchor;
        public Transform LeftLegPivot;
        public Transform RightLegPivot;
        public Transform DrillAugerPivot;
        public Animator ChassisAnimator;
        public Animator WeaponAnimator;
        public GameObject WreckSmokeVisual;
        public WorldBillboard Billboard;
        public bool IsBiped;
        public bool IsSpiderWalker;
        public bool IsRover;

        private float _walkCycleRad;
        private Vector3 _chassisSpawnLocalPos;
        private float _turretBaseLocalY;
        private float _waistRestLocalY;
        private bool _poseCaptured;
        private string _activeAnimState = string.Empty;
        private bool _isWrecked;
        private float _weaponRecoilStopAt = -1f;

        public void CaptureInitialCommercialPose()
        {
            if (ChassisTransform != null)
                _chassisSpawnLocalPos = ChassisTransform.localPosition;
            if (TurretPivot != null)
                _turretBaseLocalY = TurretPivot.localPosition.y;
            if (WaistMountSocket != null && HullTiltRoot != null)
                _waistRestLocalY = HullTiltRoot.InverseTransformPoint(WaistMountSocket.position).y;
            _poseCaptured = true;

            if (ChassisAnimator != null)
            {
                ChassisAnimator.applyRootMotion = false;
                PlayChassisState("Idle", immediate: true, animSpeed: 1f);
            }
            if (WeaponAnimator != null)
            {
                WeaponAnimator.applyRootMotion = false;
                WeaponAnimator.Play("Shoot", 0, 0f);
                WeaponAnimator.Update(0f);
                WeaponAnimator.speed = 0f;
                WeaponAnimator.enabled = false;
            }
        }

        public void PlayWeaponRecoil()
        {
            if (WeaponAnimator == null) return;
            WeaponAnimator.enabled = true;
            WeaponAnimator.speed = 1.35f;
            WeaponAnimator.Play("Shoot", 0, 0f);
            _weaponRecoilStopAt = Time.time + 0.45f;
        }

        private void PlayChassisState(string stateName, bool immediate, float animSpeed)
        {
            if (ChassisAnimator == null) return;
            ChassisAnimator.speed = animSpeed;
            if (_activeAnimState == stateName) return;
            _activeAnimState = stateName;
            if (immediate)
                ChassisAnimator.Play(stateName, 0, 0f);
            else
                ChassisAnimator.CrossFadeInFixedTime(stateName, 0.16f, 0);
        }

        public void UpdateArticulation(float dt, float speedKph, float? worldTurretAimYawDeg = null, bool drilling = false)
        {
            if (Root == null) return;

            if (WeaponAnimator != null && _weaponRecoilStopAt > 0f && Time.time >= _weaponRecoilStopAt)
            {
                _weaponRecoilStopAt = -1f;
                WeaponAnimator.Play("Shoot", 0, 0f);
                WeaponAnimator.Update(0f);
                WeaponAnimator.speed = 0f;
                WeaponAnimator.enabled = false;
            }

            if (_poseCaptured && ChassisTransform != null)
            {
                ChassisTransform.localPosition = _chassisSpawnLocalPos;
                ChassisTransform.localRotation = Quaternion.identity;
            }

            // Drive real Slava Z skeletal Idle / Walk / Run / Walk_Back clips on Biped Mechs & Spider Walkers!
            if (ChassisAnimator != null && !_isWrecked)
            {
                float absSpd = Mathf.Abs(speedKph);
                if (IsBiped)
                {
                    if (absSpd < 1.0f)
                    {
                        PlayChassisState("Idle", immediate: false, animSpeed: 1f);
                    }
                    else if (absSpd > 34f && speedKph > 0f)
                    {
                        PlayChassisState("Run", immediate: false, animSpeed: Mathf.Clamp(absSpd / 46f, 0.78f, 1.35f));
                    }
                    else
                    {
                        // Reverse steps play Walk smoothly
                        PlayChassisState("Walk", immediate: false, animSpeed: Mathf.Clamp(absSpd / 22f, 0.65f, 1.35f));
                    }
                }
                else if (IsSpiderWalker)
                {
                    if (absSpd < 1.0f)
                    {
                        PlayChassisState("Idle", immediate: false, animSpeed: 1f);
                    }
                    else if (speedKph < -0.5f)
                    {
                        PlayChassisState("Walk_Back", immediate: false, animSpeed: Mathf.Clamp(absSpd / 22f, 0.7f, 1.35f));
                    }
                    else
                    {
                        PlayChassisState("Walk", immediate: false, animSpeed: Mathf.Clamp(absSpd / 32f, 0.7f, 1.45f));
                    }
                }

                // Follow skeletal pelvis vertical stride bob on upper torso/turret while keeping yaw aim independent!
                if (_poseCaptured && WaistMountSocket != null && HullTiltRoot != null && TurretPivot != null)
                {
                    float currentWaistY = HullTiltRoot.InverseTransformPoint(WaistMountSocket.position).y;
                    float bobDeltaY = Mathf.Clamp(currentWaistY - _waistRestLocalY, -0.35f, 0.35f);
                    TurretPivot.localPosition = new Vector3(0f, _turretBaseLocalY + bobDeltaY, 0f);
                }
            }

            if (TurretPivot != null && worldTurretAimYawDeg.HasValue)
            {
                // Aim turret in local yaw relative to HullTiltRoot so hillside pitch/roll slope tilt is preserved!
                float parentYaw = TurretPivot.parent != null
                    ? TurretPivot.parent.eulerAngles.y
                    : (Root != null ? Root.transform.eulerAngles.y : 0f);
                float localYaw = Mathf.DeltaAngle(parentYaw, worldTurretAimYawDeg.Value);
                Quaternion targetLocal = Quaternion.Euler(0f, localYaw, 0f);
                TurretPivot.localRotation = Quaternion.Slerp(TurretPivot.localRotation, targetLocal, 1f - Mathf.Exp(-10f * dt));
            }
            else if (TurretPivot != null)
            {
                TurretPivot.localRotation = Quaternion.Slerp(TurretPivot.localRotation, Quaternion.identity, 1f - Mathf.Exp(-6f * dt));
            }

            // Procedural fallback biped stride (used only when commercial skeletal prefab is absent)
            if (IsBiped && ChassisAnimator == null && (LeftLegPivot != null || RightLegPivot != null))
            {
                float absSpd = Mathf.Abs(speedKph);
                if (absSpd > 0.5f)
                {
                    _walkCycleRad += dt * Mathf.Clamp(absSpd * 0.24f, 2.2f, 9.5f);
                    float swing = Mathf.Sin(_walkCycleRad) * Mathf.Clamp(absSpd * 0.65f, 8f, 28f);
                    if (LeftLegPivot != null) LeftLegPivot.localRotation = Quaternion.Euler(swing, 0f, 0f);
                    if (RightLegPivot != null) RightLegPivot.localRotation = Quaternion.Euler(-swing, 0f, 0f);
                    if (HullTiltRoot != null)
                        HullTiltRoot.localPosition = new Vector3(0f, Mathf.Abs(Mathf.Cos(_walkCycleRad)) * 0.14f, 0f);
                }
                else
                {
                    if (LeftLegPivot != null) LeftLegPivot.localRotation = Quaternion.Slerp(LeftLegPivot.localRotation, Quaternion.identity, 10f * dt);
                    if (RightLegPivot != null) RightLegPivot.localRotation = Quaternion.Slerp(RightLegPivot.localRotation, Quaternion.identity, 10f * dt);
                    if (HullTiltRoot != null) HullTiltRoot.localPosition = Vector3.Lerp(HullTiltRoot.localPosition, Vector3.zero, 10f * dt);
                }
            }

            if (DrillAugerPivot != null)
            {
                float targetPitch = drilling ? 38f : 0f;
                DrillAugerPivot.localRotation = Quaternion.Slerp(DrillAugerPivot.localRotation, Quaternion.Euler(targetPitch, 0f, 0f), 8f * dt);
            }
        }

        public void SetWrecked(bool wrecked)
        {
            if (WreckSmokeVisual != null && WreckSmokeVisual.activeSelf != wrecked)
                WreckSmokeVisual.SetActive(wrecked);

            if (wrecked && !_isWrecked)
            {
                _isWrecked = true;
                if (ChassisAnimator != null)
                {
                    if (IsBiped) PlayChassisState("Fall_forward", immediate: false, animSpeed: 1f);
                    else if (IsSpiderWalker) PlayChassisState("Death", immediate: false, animSpeed: 1f);
                }
            }
            else if (!wrecked && _isWrecked)
            {
                _isWrecked = false;
                if (ChassisAnimator != null)
                    PlayChassisState("Idle", immediate: true, animSpeed: 1f);
            }
        }
    }

    /// <summary>
    /// Spawns commercial Slava Z Tanks / Biped Mechs / Rovers when imported in TacSimAssetCatalog,
    /// augmented with team-colored Red/Blue chevrons, articulated Turret/Muzzle/Cupola anchors,
    /// and sculpted URP PBR procedural armor rigs as zero-dependency fallbacks.
    /// </summary>
    public static class TacSimVehicleVisualFactory
    {
        public static SpawnedGroundVehicleRig SpawnArenaCombatUnit(
            TacSimAssetCatalog catalog,
            Transform parent,
            string unitType,
            string callsign,
            string party,
            Vector3 worldPos,
            float headingDeg)
        {
            string u = (unitType ?? "tacsim.unit.line_tank").ToLowerInvariant();
            bool isBiped = u.Contains("biped") || u.Contains("mech");
            bool isHeavy = u.Contains("heavy");
            bool isScout = u.Contains("scout") || u.Contains("escort");

            bool isRed = string.Equals(party, "R", StringComparison.OrdinalIgnoreCase) ||
                         string.Equals(party, "red", StringComparison.OrdinalIgnoreCase);
            bool isBlue = string.Equals(party, "B", StringComparison.OrdinalIgnoreCase) ||
                          string.Equals(party, "blue", StringComparison.OrdinalIgnoreCase);

            Color teamMain = isRed ? new Color(0.84f, 0.22f, 0.18f)
                           : isBlue ? new Color(0.18f, 0.48f, 0.90f)
                           : new Color(0.75f, 0.65f, 0.22f);
            Color teamGlow = teamMain * 1.8f;

            var root = new GameObject($"Unit_{callsign}_{u}");
            root.transform.SetParent(parent, false);
            root.transform.position = worldPos;
            root.transform.rotation = Quaternion.Euler(0f, headingDeg, 0f);

            // Click-to-lock BoxCollider on root
            var col = root.AddComponent<BoxCollider>();
            col.size = isBiped ? new Vector3(3.4f, 5.2f, 3.4f) : new Vector3(4.2f, 2.8f, 6.4f);
            col.center = new Vector3(0f, col.size.y * 0.5f, 0f);

            var tilt = new GameObject("HullRoot").transform;
            tilt.SetParent(root.transform, false);

            var rig = new SpawnedGroundVehicleRig
            {
                Root = root,
                HullTiltRoot = tilt,
                IsBiped = isBiped,
                IsSpiderWalker = isScout
            };

            GameObject commercialPrefab = null;
            if (catalog != null)
            {
                if (isBiped) commercialPrefab = catalog.bipedMechPrefab;
                else if (isHeavy) commercialPrefab = catalog.heavyTankPrefab != null ? catalog.heavyTankPrefab : catalog.lineTankPrefab;
                else if (isScout) commercialPrefab = catalog.scoutWalkerPrefab != null ? catalog.scoutWalkerPrefab : catalog.lineTankPrefab;
                else commercialPrefab = catalog.lineTankPrefab;
            }

            if (commercialPrefab != null)
            {
                Texture teamTex = null;
                if (catalog != null)
                {
                    if (isBiped) teamTex = isRed ? catalog.ltMedRedAlbedo : (isBlue ? catalog.ltMedBlueAlbedo : null);
                    else teamTex = isRed ? catalog.spidersRedAlbedo : (isBlue ? catalog.spidersBlueAlbedo : null);
                }

                float chassisH = isBiped ? 3.1f : (isHeavy ? 1.85f : 1.55f);
                float maxDim = isBiped ? 4.4f : (isHeavy ? 7.6f : 6.4f);
                var chassisInst = PrefabFittingHelper.SpawnGrounded(commercialPrefab, tilt, "CommercialChassis", worldPos, root.transform.rotation, chassisH, maxDim, keepColliders: false, overrideAlbedo: teamTex);
                if (chassisInst != null)
                {
                    rig.ChassisTransform = chassisInst.transform;
                    rig.ChassisAnimator = chassisInst.GetComponentInChildren<Animator>();
                    rig.WaistMountSocket = FindChildSocketIgnoreCase(chassisInst.transform, "Mount_top");
                }

                var turret = new GameObject("TurretMount").transform;
                turret.SetParent(tilt, false);
                float turretMountY = rig.WaistMountSocket != null
                    ? tilt.InverseTransformPoint(rig.WaistMountSocket.position).y
                    : chassisH * 0.88f;
                turret.localPosition = new Vector3(0f, turretMountY, 0f);
                rig.TurretPivot = turret;

                GameObject turretPrefab = null;
                if (catalog != null)
                {
                    if (isBiped) turretPrefab = catalog.bipedCockpitPrefab;
                    else if (isHeavy && catalog.tankTurretTwinPrefab != null) turretPrefab = catalog.tankTurretTwinPrefab;
                    else turretPrefab = catalog.tankTurretSinglePrefab;
                }

                if (turretPrefab != null)
                {
                    PrefabFittingHelper.SpawnGrounded(turretPrefab, turret, "CommercialTurret", turret.position, root.transform.rotation, isBiped ? 2.1f : 1.55f, 4.2f, keepColliders: false, overrideAlbedo: teamTex);
                    if (isBiped && catalog != null && catalog.bipedCannonPrefab != null)
                    {
                        var cannonInst = PrefabFittingHelper.SpawnCentered(catalog.bipedCannonPrefab, turret, "ShoulderCannon", turret.position + root.transform.right * 1.35f + Vector3.up * 0.9f, root.transform.rotation, 2.4f, keepColliders: false, overrideAlbedo: teamTex);
                        if (cannonInst != null)
                            rig.WeaponAnimator = cannonInst.GetComponentInChildren<Animator>();
                    }
                }

                var muzzle = new GameObject("MuzzlePoint").transform;
                muzzle.SetParent(turret, false);
                muzzle.localPosition = new Vector3(0f, 0.55f, 3.2f);
                rig.MuzzlePoint = muzzle;

                rig.CaptureInitialCommercialPose();
            }
            else if (isBiped)
            {
                BuildSculptedBipedMech(rig, tilt, teamMain, teamGlow);
            }
            else
            {
                BuildSculptedCombatTank(rig, tilt, teamMain, teamGlow, isHeavy, isScout);
            }

            // Commander cupola camera anchor
            var cupola = new GameObject("CupolaAnchor").transform;
            cupola.SetParent(rig.TurretPivot != null ? rig.TurretPivot : tilt, false);
            cupola.localPosition = isBiped ? new Vector3(0f, 0.85f, 0.65f) : new Vector3(0f, 1.15f, -0.35f);
            rig.CupolaCameraAnchor = cupola;

            // Wreck fire/smoke visual (hidden until wrecked)
            rig.WreckSmokeVisual = BuildWreckMarker(tilt, catalog);
            rig.WreckSmokeVisual.SetActive(false);

            // Tactical floating nameplate & armor/health status
            var bbGo = new GameObject("TacticalTag");
            bbGo.transform.SetParent(root.transform, false);
            bbGo.transform.localPosition = new Vector3(0f, isBiped ? 6.2f : 4.2f, 0f);
            var bb = bbGo.AddComponent<WorldBillboard>();
            Color tagCol = isRed ? new Color(1f, 0.45f, 0.40f) : isBlue ? new Color(0.45f, 0.78f, 1f) : new Color(1f, 0.90f, 0.45f);
            bb.NameplateText = CreateTmp(bbGo.transform, "TagText", Vector3.zero, callsign, 5.2f, tagCol);
            rig.Billboard = bb;

            return rig;
        }

        public static SpawnedGroundVehicleRig SpawnSurfaceRover(
            TacSimAssetCatalog catalog,
            Transform parent,
            string label,
            Vector3 worldPos,
            float headingDeg,
            bool isEscort = false)
        {
            var root = new GameObject($"Rover_{label}");
            root.transform.SetParent(parent, false);
            root.transform.position = worldPos;
            root.transform.rotation = Quaternion.Euler(0f, headingDeg, 0f);

            var tilt = new GameObject("RoverChassis").transform;
            tilt.SetParent(root.transform, false);

            var rig = new SpawnedGroundVehicleRig
            {
                Root = root,
                HullTiltRoot = tilt,
                IsRover = true
            };

            GameObject prefab = catalog != null
                ? (isEscort && catalog.escortVehiclePrefab != null ? catalog.escortVehiclePrefab : catalog.prospectorRoverPrefab)
                : null;

            Color bodyCol = isEscort ? new Color(0.32f, 0.38f, 0.44f) : new Color(0.78f, 0.54f, 0.14f);
            Material bodyMat = UrpMaterialFactory.GetOpaque(isEscort ? "EscortArmor" : "RoverAmber", bodyCol, 0.48f, 0.45f);
            Material darkMat = UrpMaterialFactory.GetOpaque("RoverFrame", new Color(0.16f, 0.18f, 0.21f), 0.35f, 0.65f);
            Material cabGlass = UrpMaterialFactory.GetEmissive("RoverCabVisor", new Color(0.15f, 0.45f, 0.65f), new Color(0.25f, 0.82f, 1f) * 1.1f);

            if (prefab != null)
            {
                PrefabFittingHelper.SpawnGrounded(prefab, tilt, "CommercialRover", worldPos, root.transform.rotation, 2.4f, 5.4f, keepColliders: false);
            }
            else
            {
                // 6-wheel articulated Co-op Prospector Rig (`tv_prospector`)
                CreatePart(tilt, "LowerDeckFrame", new Vector3(0f, 0.72f, 0f), new Vector3(2.5f, 0.55f, 5.0f), darkMat);
                CreatePart(tilt, "ForwardCab", new Vector3(0f, 1.48f, 1.35f), new Vector3(2.2f, 1.05f, 1.85f), bodyMat);
                CreatePart(tilt, "CabWindscreen", new Vector3(0f, 1.58f, 2.28f), new Vector3(1.85f, 0.52f, 0.12f), cabGlass);
                CreatePart(tilt, "OreHopperBin", new Vector3(0f, 1.45f, -1.15f), new Vector3(2.35f, 1.0f, 2.45f), bodyMat);

                float[] wheelZs = { -1.75f, 0f, 1.75f };
                for (int i = 0; i < 3; i++)
                {
                    CreateWheel(tilt, $"Wheel_L{i}", new Vector3(-1.42f, 0.52f, wheelZs[i]), darkMat);
                    CreateWheel(tilt, $"Wheel_R{i}", new Vector3(1.42f, 0.52f, wheelZs[i]), darkMat);
                }
            }

            // Articulated bow core-drill auger & seismic sounder mast
            var augerPivot = new GameObject("CoreDrillAuger").transform;
            augerPivot.SetParent(tilt, false);
            augerPivot.localPosition = new Vector3(0f, 0.95f, 2.45f);
            CreatePart(augerPivot, "DrillBoom", new Vector3(0f, 0f, 0.65f), new Vector3(0.32f, 0.32f, 1.35f), darkMat);
            CreatePart(augerPivot, "AugerBit", new Vector3(0f, -0.25f, 1.25f), new Vector3(0.24f, 0.75f, 0.24f), cabGlass);
            rig.DrillAugerPivot = augerPivot;

            var mast = new GameObject("SensorTurret").transform;
            mast.SetParent(tilt, false);
            mast.localPosition = new Vector3(0f, 2.15f, 1.15f);
            CreatePart(mast, "RadarDish", new Vector3(0f, 0.2f, 0f), new Vector3(0.85f, 0.25f, 0.35f), cabGlass);
            rig.TurretPivot = mast;

            var cupola = new GameObject("CabCameraAnchor").transform;
            cupola.SetParent(tilt, false);
            cupola.localPosition = new Vector3(0f, 2.15f, 1.55f);
            rig.CupolaCameraAnchor = cupola;

            var bbGo = new GameObject("RoverTag");
            bbGo.transform.SetParent(root.transform, false);
            bbGo.transform.localPosition = new Vector3(0f, 3.4f, 0f);
            var bb = bbGo.AddComponent<WorldBillboard>();
            bb.NameplateText = CreateTmp(bbGo.transform, "TagText", Vector3.zero, label, 3.8f, new Color(1f, 0.88f, 0.45f));
            rig.Billboard = bb;

            return rig;
        }

        private static void BuildSculptedCombatTank(SpawnedGroundVehicleRig rig, Transform parent, Color teamMain, Color teamGlow, bool isHeavy, bool isScout)
        {
            float scale = isHeavy ? 1.22f : (isScout ? 0.84f : 1.0f);
            Material hullMat = UrpMaterialFactory.GetOpaque($"TankHull_{teamMain}", Color.Lerp(new Color(0.24f, 0.26f, 0.29f), teamMain, 0.35f), 0.42f, 0.58f);
            Material trackMat = UrpMaterialFactory.GetOpaque("TankTracks", new Color(0.13f, 0.14f, 0.16f), 0.25f, 0.5f);
            Material gunMat = UrpMaterialFactory.GetOpaque("TankBarrel", new Color(0.20f, 0.22f, 0.25f), 0.55f, 0.78f);
            Material stripeMat = UrpMaterialFactory.GetEmissive($"TankStripe_{teamMain}", teamMain, teamGlow);

            // Lower chassis + armored track sponsons
            CreatePart(parent, "GlacisHull", new Vector3(0f, 0.92f, 0f) * scale, new Vector3(2.6f, 0.95f, 5.6f) * scale, hullMat);
            CreatePart(parent, "TrackPod_L", new Vector3(-1.58f, 0.62f, 0f) * scale, new Vector3(0.75f, 1.05f, 5.9f) * scale, trackMat);
            CreatePart(parent, "TrackPod_R", new Vector3(1.58f, 0.62f, 0f) * scale, new Vector3(0.75f, 1.05f, 5.9f) * scale, trackMat);
            CreatePart(parent, "TeamSkirt_L", new Vector3(-1.98f, 0.85f, 0f) * scale, new Vector3(0.10f, 0.36f, 4.8f) * scale, stripeMat);
            CreatePart(parent, "TeamSkirt_R", new Vector3(1.98f, 0.85f, 0f) * scale, new Vector3(0.10f, 0.36f, 4.8f) * scale, stripeMat);

            // Traversing turret
            var turret = new GameObject("TurretPivot").transform;
            turret.SetParent(parent, false);
            turret.localPosition = new Vector3(0f, 1.58f * scale, -0.15f * scale);
            rig.TurretPivot = turret;

            CreatePart(turret, "TurretBustle", new Vector3(0f, 0.25f, -0.25f) * scale, new Vector3(2.25f, 0.78f, 2.85f) * scale, hullMat);
            CreatePart(turret, "TurretChevron", new Vector3(0f, 0.68f, -0.25f) * scale, new Vector3(1.45f, 0.12f, 1.65f) * scale, stripeMat);

            if (isHeavy)
            {
                // Twin heavy cannons!
                CreateBarrel(turret, "MainCannon_L", new Vector3(-0.38f, 0.26f, 2.25f) * scale, 0.16f * scale, 2.1f * scale, gunMat);
                CreateBarrel(turret, "MainCannon_R", new Vector3(0.38f, 0.26f, 2.25f) * scale, 0.16f * scale, 2.1f * scale, gunMat);
            }
            else
            {
                CreateBarrel(turret, "MainCannon", new Vector3(0f, 0.26f, 2.15f) * scale, 0.15f * scale, 2.0f * scale, gunMat);
            }

            var muzzle = new GameObject("MuzzlePoint").transform;
            muzzle.SetParent(turret, false);
            muzzle.localPosition = new Vector3(0f, 0.26f * scale, 4.15f * scale);
            rig.MuzzlePoint = muzzle;
        }

        private static void BuildSculptedBipedMech(SpawnedGroundVehicleRig rig, Transform parent, Color teamMain, Color teamGlow)
        {
            Material armorMat = UrpMaterialFactory.GetOpaque($"MechArmor_{teamMain}", Color.Lerp(new Color(0.25f, 0.27f, 0.31f), teamMain, 0.40f), 0.48f, 0.62f);
            Material jointMat = UrpMaterialFactory.GetOpaque("MechJoint", new Color(0.15f, 0.17f, 0.20f), 0.35f, 0.7f);
            Material visorMat = UrpMaterialFactory.GetEmissive($"MechVisor_{teamMain}", teamMain, teamGlow * 1.2f);

            CreatePart(parent, "PelvisBlock", new Vector3(0f, 2.35f, 0f), new Vector3(1.65f, 0.55f, 1.25f), jointMat);

            var legL = new GameObject("LeftLegPivot").transform;
            legL.SetParent(parent, false);
            legL.localPosition = new Vector3(-0.95f, 2.25f, 0f);
            CreatePart(legL, "Thigh_L", new Vector3(0f, -0.65f, 0.15f), new Vector3(0.55f, 1.25f, 0.65f), armorMat);
            CreatePart(legL, "Shin_L", new Vector3(0f, -1.65f, -0.10f), new Vector3(0.48f, 1.20f, 0.58f), jointMat);
            CreatePart(legL, "Foot_L", new Vector3(0f, -2.15f, 0.18f), new Vector3(0.82f, 0.25f, 1.25f), armorMat);
            rig.LeftLegPivot = legL;

            var legR = new GameObject("RightLegPivot").transform;
            legR.SetParent(parent, false);
            legR.localPosition = new Vector3(0.95f, 2.25f, 0f);
            CreatePart(legR, "Thigh_R", new Vector3(0f, -0.65f, 0.15f), new Vector3(0.55f, 1.25f, 0.65f), armorMat);
            CreatePart(legR, "Shin_R", new Vector3(0f, -1.65f, -0.10f), new Vector3(0.48f, 1.20f, 0.58f), jointMat);
            CreatePart(legR, "Foot_R", new Vector3(0f, -2.15f, 0.18f), new Vector3(0.82f, 0.25f, 1.25f), armorMat);
            rig.RightLegPivot = legR;

            var torso = new GameObject("TorsoTurret").transform;
            torso.SetParent(parent, false);
            torso.localPosition = new Vector3(0f, 2.75f, 0f);
            rig.TurretPivot = torso;

            CreatePart(torso, "CockpitPod", new Vector3(0f, 0.85f, 0.15f), new Vector3(2.15f, 1.55f, 2.25f), armorMat);
            CreatePart(torso, "CockpitVisor", new Vector3(0f, 0.95f, 1.28f), new Vector3(1.35f, 0.42f, 0.16f), visorMat);
            CreatePart(torso, "MissilePod_L", new Vector3(-1.45f, 1.45f, 0.1f), new Vector3(0.68f, 0.78f, 1.45f), visorMat);
            CreatePart(torso, "Autocannon_R", new Vector3(1.52f, 0.75f, 0.85f), new Vector3(0.45f, 0.45f, 2.35f), jointMat);

            var muzzle = new GameObject("MuzzlePoint").transform;
            muzzle.SetParent(torso, false);
            muzzle.localPosition = new Vector3(1.52f, 0.75f, 2.2f);
            rig.MuzzlePoint = muzzle;
        }

        private static GameObject BuildWreckMarker(Transform parent, TacSimAssetCatalog catalog)
        {
            var wRoot = new GameObject("WreckFireMarker");
            wRoot.transform.SetParent(parent, false);
            wRoot.transform.localPosition = new Vector3(0f, 1.6f, 0f);

            if (catalog != null && catalog.shipVentMistPrefab != null)
            {
                var smoke = UnityEngine.Object.Instantiate(catalog.shipVentMistPrefab, wRoot.transform);
                smoke.transform.localPosition = Vector3.zero;
                smoke.transform.localScale = Vector3.one * 2.5f;
            }

            Material emberMat = UrpMaterialFactory.GetEmissive("WreckEmber", new Color(0.9f, 0.3f, 0.08f), new Color(1.0f, 0.38f, 0.05f) * 2.4f);
            CreatePart(wRoot.transform, "WreckCoreGlow", Vector3.zero, new Vector3(1.2f, 1.2f, 1.2f), emberMat);
            return wRoot;
        }

        private static GameObject CreatePart(Transform parent, string name, Vector3 localPos, Vector3 scale, Material mat)
        {
            var go = GameObject.CreatePrimitive(PrimitiveType.Cube);
            go.name = name;
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            go.transform.localScale = scale;
            var col = go.GetComponent<Collider>();
            if (col != null)
            {
                if (Application.isPlaying) UnityEngine.Object.Destroy(col);
                else UnityEngine.Object.DestroyImmediate(col);
            }
            go.GetComponent<MeshRenderer>().sharedMaterial = mat;
            return go;
        }

        private static void CreateBarrel(Transform parent, string name, Vector3 localPos, float radius, float halfLen, Material mat)
        {
            var go = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            go.name = name;
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            go.transform.localRotation = Quaternion.Euler(90f, 0f, 0f);
            go.transform.localScale = new Vector3(radius * 2f, halfLen, radius * 2f);
            var col = go.GetComponent<Collider>();
            if (col != null)
            {
                if (Application.isPlaying) UnityEngine.Object.Destroy(col);
                else UnityEngine.Object.DestroyImmediate(col);
            }
            go.GetComponent<MeshRenderer>().sharedMaterial = mat;
        }

        private static void CreateWheel(Transform parent, string name, Vector3 localPos, Material mat)
        {
            var go = GameObject.CreatePrimitive(PrimitiveType.Cylinder);
            go.name = name;
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            go.transform.localRotation = Quaternion.Euler(0f, 0f, 90f);
            go.transform.localScale = new Vector3(1.02f, 0.26f, 1.02f);
            var col = go.GetComponent<Collider>();
            if (col != null)
            {
                if (Application.isPlaying) UnityEngine.Object.Destroy(col);
                else UnityEngine.Object.DestroyImmediate(col);
            }
            go.GetComponent<MeshRenderer>().sharedMaterial = mat;
        }

        private static Transform FindChildSocketIgnoreCase(Transform root, string targetName)
        {
            if (root == null) return null;
            for (int i = 0; i < root.childCount; i++)
            {
                var ch = root.GetChild(i);
                if (string.Equals(ch.name, targetName, StringComparison.OrdinalIgnoreCase))
                    return ch;
                var nested = FindChildSocketIgnoreCase(ch, targetName);
                if (nested != null)
                    return nested;
            }
            return null;
        }

        private static TextMeshPro CreateTmp(Transform parent, string name, Vector3 localPos, string text, float fontSize, Color color)
        {
            var go = new GameObject(name);
            go.transform.SetParent(parent, false);
            go.transform.localPosition = localPos;
            var tmp = go.AddComponent<TextMeshPro>();
            tmp.text = text;
            tmp.fontSize = fontSize;
            tmp.color = color;
            tmp.alignment = TextAlignmentOptions.Center;
            tmp.textWrappingMode = TextWrappingModes.NoWrap;
            tmp.rectTransform.sizeDelta = new Vector2(14f, 3.2f);
            return tmp;
        }
    }
}
