using UnityEngine;
using UnityEngine.EventSystems;

namespace TacSim.Character
{
    public enum CameraRigMode
    {
        ThirdPersonWalk = 0,
        ShipBridgeHolo = 1,
        OrbitalExterior = 2,
        BowDockingCollar = 3,
        GroundVehicleChase = 4,
        GroundVehicleCupola = 5,
        OrbitalTacticalHigh = 6
    }

    /// <summary>
    /// Unified 6-mode camera rig:
    /// 1) Wall-avoiding 3rd-person over-shoulder orbit camera (stations, OOC Sim Centre &amp; ship compartments)
    /// 2) Bridge Holo-Tank tactical overview
    /// 3) 3D Orbital Exterior chase camera around the hull in space
    /// 4) 60 Hz 6-DOF Bow Docking Collar alignment camera
    /// 5) 3D Ground Vehicle Chase camera (Surface Prospector Rovers &amp; Red vs Blue Arena Tanks/Mechs)
    /// 6) Commander Cupola / Cockpit visor view
    /// </summary>
    public sealed class TacSimCameraRig : MonoBehaviour
    {
        public Camera ControlledCamera;
        public Transform WalkFollowTarget;
        public Transform BridgeViewAnchor;
        public Transform OrbitalShipTarget;
        public Transform BowDockingAnchor;
        public Transform GroundVehicleTarget;
        public Transform GroundCupolaAnchor;

        [Header("3rd Person Orbit")]
        public Vector3 shoulderOffset = new Vector3(0.42f, 1.58f, 0f);
        public float walkDistance = 3.6f;
        public float minWalkDistance = 1.6f;
        public float maxWalkDistance = 6.5f;
        public float orbitalDistance = 26f;
        public float orbitalTacticalDistance = 138f;
        public float vehicleDistance = 18f;
        public float mouseSensitivity = 2.8f;

        public CameraRigMode Mode { get; private set; } = CameraRigMode.ThirdPersonWalk;
        public bool OrbitInputEnabled { get; set; } = true;

        private float _yawDeg = 0f;
        private float _pitchDeg = 16f;
        private float _orbitalYawDeg = 25f;
        private float _orbitalPitchDeg = 20f;
        private float _tacticalYawDeg = 32f;
        private float _tacticalPitchDeg = 52f;
        private float _vehicleYawOffsetDeg = 0f;
        private float _vehiclePitchDeg = 22f;
        private Vector3 _smoothedPivot;
        private bool _pivotInitialized;
        private bool _rmbOrbitDragging;

        private bool UpdateRmbOrbitDragState(bool overUi)
        {
            if (!OrbitInputEnabled)
            {
                _rmbOrbitDragging = false;
                return false;
            }
            if (Input.GetMouseButtonDown(1))
                _rmbOrbitDragging = !overUi;
            if (!Input.GetMouseButton(1))
                _rmbOrbitDragging = false;
            return _rmbOrbitDragging;
        }

        private void Awake()
        {
            if (ControlledCamera == null)
                ControlledCamera = GetComponentInChildren<Camera>();
        }

        public void SetMode(CameraRigMode mode)
        {
            Mode = mode;
            if (ControlledCamera != null)
            {
                ControlledCamera.nearClipPlane = mode == CameraRigMode.BowDockingCollar ? 0.05f : 0.15f;
                ControlledCamera.farClipPlane = (mode == CameraRigMode.OrbitalExterior || mode == CameraRigMode.OrbitalTacticalHigh || mode == CameraRigMode.GroundVehicleChase || mode == CameraRigMode.GroundVehicleCupola) ? 8000f : 1500f;
                ControlledCamera.fieldOfView = mode == CameraRigMode.BowDockingCollar ? 66f : 62f;
            }
        }

        public void SnapBehindTarget(float yawDeg)
        {
            _yawDeg = yawDeg;
            _pitchDeg = 16f;
            _vehicleYawOffsetDeg = 0f;
            if (WalkFollowTarget != null)
            {
                _smoothedPivot = WalkFollowTarget.position + Vector3.up * shoulderOffset.y;
                _pivotInitialized = true;
            }
        }

        private void LateUpdate()
        {
            if (ControlledCamera == null) return;
            float dt = Mathf.Max(0.0001f, Time.deltaTime);

            switch (Mode)
            {
                case CameraRigMode.ThirdPersonWalk:
                    UpdateThirdPersonWalk(dt);
                    break;

                case CameraRigMode.ShipBridgeHolo:
                    if (BridgeViewAnchor != null)
                    {
                        ControlledCamera.transform.position = Vector3.Lerp(ControlledCamera.transform.position, BridgeViewAnchor.position, 1f - Mathf.Exp(-8f * dt));
                        ControlledCamera.transform.rotation = Quaternion.Slerp(ControlledCamera.transform.rotation, BridgeViewAnchor.rotation, 1f - Mathf.Exp(-8f * dt));
                    }
                    break;

                case CameraRigMode.OrbitalExterior:
                    UpdateOrbitalChase(dt);
                    break;

                case CameraRigMode.OrbitalTacticalHigh:
                    UpdateOrbitalTacticalHigh(dt);
                    break;

                case CameraRigMode.BowDockingCollar:
                    if (BowDockingAnchor != null)
                    {
                        ControlledCamera.transform.SetPositionAndRotation(BowDockingAnchor.position, BowDockingAnchor.rotation);
                    }
                    break;

                case CameraRigMode.GroundVehicleChase:
                    UpdateGroundVehicleChase(dt);
                    break;

                case CameraRigMode.GroundVehicleCupola:
                    if (GroundCupolaAnchor != null)
                    {
                        ControlledCamera.transform.position = Vector3.Lerp(ControlledCamera.transform.position, GroundCupolaAnchor.position, 1f - Mathf.Exp(-18f * dt));
                        ControlledCamera.transform.rotation = Quaternion.Slerp(ControlledCamera.transform.rotation, GroundCupolaAnchor.rotation, 1f - Mathf.Exp(-18f * dt));
                    }
                    else
                    {
                        UpdateGroundVehicleChase(dt);
                    }
                    break;
            }
        }

        private void UpdateThirdPersonWalk(float dt)
        {
            if (WalkFollowTarget == null) return;

            bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
            if (OrbitInputEnabled)
            {
                if (UpdateRmbOrbitDragState(overUi))
                {
                    _yawDeg += Input.GetAxis("Mouse X") * mouseSensitivity * 1.6f;
                    _pitchDeg = Mathf.Clamp(_pitchDeg - Input.GetAxis("Mouse Y") * mouseSensitivity * 1.3f, -22f, 68f);
                }

                float scroll = Input.mouseScrollDelta.y;
                if (!overUi && Mathf.Abs(scroll) > 0.01f)
                {
                    walkDistance = Mathf.Clamp(walkDistance - scroll * 0.45f, minWalkDistance, maxWalkDistance);
                }
            }
            else
            {
                _rmbOrbitDragging = false;
            }

            Quaternion orbitRot = Quaternion.Euler(_pitchDeg, _yawDeg, 0f);
            Vector3 headCenter = WalkFollowTarget.position + Vector3.up * shoulderOffset.y;
            Vector3 shoulderDelta = Quaternion.Euler(0f, _yawDeg, 0f) * new Vector3(shoulderOffset.x, 0f, 0f);
            Vector3 desiredPivot = headCenter + shoulderDelta;
            float shoulderLen = shoulderDelta.magnitude;
            if (shoulderLen > 0.01f &&
                Physics.SphereCast(headCenter, 0.16f, shoulderDelta / shoulderLen, out RaycastHit shoulderHit, shoulderLen, ~0, QueryTriggerInteraction.Ignore))
            {
                if (shoulderHit.transform != WalkFollowTarget && !shoulderHit.transform.IsChildOf(WalkFollowTarget))
                {
                    float clampedShoulder = Mathf.Max(0f, shoulderHit.distance - 0.12f);
                    desiredPivot = headCenter + (shoulderDelta / shoulderLen) * clampedShoulder;
                }
            }

            if (!_pivotInitialized)
            {
                _smoothedPivot = desiredPivot;
                _pivotInitialized = true;
            }
            else
            {
                _smoothedPivot = Vector3.Lerp(_smoothedPivot, desiredPivot, 1f - Mathf.Exp(-16f * dt));
            }

            Vector3 backDir = orbitRot * Vector3.back;
            float safeDist = walkDistance;

            // Spherecast to prevent clipping into station bulkheads or narrow ship passageways
            if (Physics.SphereCast(_smoothedPivot, 0.22f, backDir, out RaycastHit hit, walkDistance, ~0, QueryTriggerInteraction.Ignore))
            {
                if (hit.transform != WalkFollowTarget && !hit.transform.IsChildOf(WalkFollowTarget))
                {
                    safeDist = Mathf.Max(0.65f, hit.distance - 0.15f);
                }
            }

            ControlledCamera.transform.position = _smoothedPivot + backDir * safeDist;
            ControlledCamera.transform.rotation = orbitRot;
        }

        private void UpdateOrbitalChase(float dt)
        {
            if (OrbitalShipTarget == null) return;

            bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
            if (OrbitInputEnabled)
            {
                if (UpdateRmbOrbitDragState(overUi))
                {
                    _orbitalYawDeg += Input.GetAxis("Mouse X") * mouseSensitivity * 1.8f;
                    _orbitalPitchDeg = Mathf.Clamp(_orbitalPitchDeg - Input.GetAxis("Mouse Y") * mouseSensitivity * 1.4f, -55f, 75f);
                }
                float scroll = Input.mouseScrollDelta.y;
                if (!overUi && Mathf.Abs(scroll) > 0.01f)
                {
                    orbitalDistance = Mathf.Clamp(orbitalDistance - scroll * 2.5f, 10f, 110f);
                }
            }
            else
            {
                _rmbOrbitDragging = false;
            }

            Quaternion rot = Quaternion.Euler(_orbitalPitchDeg, _orbitalYawDeg, 0f);
            Vector3 desiredPos = OrbitalShipTarget.position + rot * (Vector3.back * orbitalDistance) + Vector3.up * 2.2f;
            ControlledCamera.transform.position = Vector3.Lerp(ControlledCamera.transform.position, desiredPos, 1f - Mathf.Exp(-10f * dt));
            Vector3 toShip = OrbitalShipTarget.position - ControlledCamera.transform.position;
            if (toShip.sqrMagnitude > 1e-6f)
                ControlledCamera.transform.rotation = Quaternion.LookRotation(toShip.normalized, Vector3.up);
        }

        private void UpdateOrbitalTacticalHigh(float dt)
        {
            if (OrbitalShipTarget == null) return;

            bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
            if (OrbitInputEnabled)
            {
                if (UpdateRmbOrbitDragState(overUi))
                {
                    _tacticalYawDeg += Input.GetAxis("Mouse X") * mouseSensitivity * 1.6f;
                    _tacticalPitchDeg = Mathf.Clamp(_tacticalPitchDeg - Input.GetAxis("Mouse Y") * mouseSensitivity * 1.2f, 18f, 82f);
                }
                float scroll = Input.mouseScrollDelta.y;
                if (!overUi && Mathf.Abs(scroll) > 0.01f)
                {
                    orbitalTacticalDistance = Mathf.Clamp(orbitalTacticalDistance - scroll * 8.5f, 52f, 260f);
                }
            }
            else
            {
                _rmbOrbitDragging = false;
            }

            Quaternion rot = Quaternion.Euler(_tacticalPitchDeg, _tacticalYawDeg, 0f);
            Vector3 desiredPos = OrbitalShipTarget.position + rot * (Vector3.back * orbitalTacticalDistance);
            ControlledCamera.transform.position = Vector3.Lerp(ControlledCamera.transform.position, desiredPos, 1f - Mathf.Exp(-8f * dt));
            Vector3 toShip = OrbitalShipTarget.position - ControlledCamera.transform.position;
            if (toShip.sqrMagnitude > 1e-6f)
                ControlledCamera.transform.rotation = Quaternion.LookRotation(toShip.normalized, Vector3.up);
        }

        private void UpdateGroundVehicleChase(float dt)
        {
            if (GroundVehicleTarget == null) return;

            bool overUi = EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
            if (OrbitInputEnabled)
            {
                if (UpdateRmbOrbitDragState(overUi))
                {
                    _vehicleYawOffsetDeg += Input.GetAxis("Mouse X") * mouseSensitivity * 1.8f;
                    _vehiclePitchDeg = Mathf.Clamp(_vehiclePitchDeg - Input.GetAxis("Mouse Y") * mouseSensitivity * 1.3f, 5f, 72f);
                }
                float scroll = Input.mouseScrollDelta.y;
                if (!overUi && Mathf.Abs(scroll) > 0.01f)
                {
                    vehicleDistance = Mathf.Clamp(vehicleDistance - scroll * 2.2f, 6.5f, 85f);
                }
            }
            else
            {
                _rmbOrbitDragging = false;
            }

            float hullYaw = GroundVehicleTarget.eulerAngles.y + _vehicleYawOffsetDeg;
            Quaternion orbitRot = Quaternion.Euler(_vehiclePitchDeg, hullYaw, 0f);
            Vector3 pivot = GroundVehicleTarget.position + Vector3.up * 3.1f;
            Vector3 backDir = orbitRot * Vector3.back;
            float safeDist = vehicleDistance;

            if (Physics.SphereCast(pivot, 0.35f, backDir, out RaycastHit hit, vehicleDistance, ~0, QueryTriggerInteraction.Ignore))
            {
                if (hit.transform != GroundVehicleTarget && !hit.transform.IsChildOf(GroundVehicleTarget))
                {
                    safeDist = Mathf.Max(2.2f, hit.distance - 0.35f);
                }
            }

            Vector3 desiredPos = pivot + backDir * safeDist;
            ControlledCamera.transform.position = Vector3.Lerp(ControlledCamera.transform.position, desiredPos, 1f - Mathf.Exp(-14f * dt));
            Vector3 toPivot = pivot - ControlledCamera.transform.position;
            if (toPivot.sqrMagnitude > 1e-6f)
                ControlledCamera.transform.rotation = Quaternion.LookRotation(toPivot.normalized, Vector3.up);
        }
    }
}
