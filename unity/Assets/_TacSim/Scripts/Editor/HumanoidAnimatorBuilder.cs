using System.IO;
using UnityEditor;
using UnityEditor.Animations;
using UnityEngine;

namespace TacSim.Editor
{
    /// <summary>
    /// Generates Assets/_TacSim/Generated/TacSimHumanoid.controller + loopable Unity Humanoid muscle-curve
    /// clips (Idle, Walk, Run, SeatedHelm), binding Ida Faber's authored Idle clip when imported.
    /// Works identically across all Humanoid character rigs (Stellar Trooper, Cyberpunk Street Fighter,
    /// Techwear Girls, Stellar Girl Astra, Stellar Girl Celeste).
    /// </summary>
    public static class HumanoidAnimatorBuilder
    {
        public const string GeneratedFolder = "Assets/_TacSim/Generated";
        public const string ControllerPath = GeneratedFolder + "/TacSimHumanoid.controller";

        public static RuntimeAnimatorController BuildOrRefreshController()
        {
            EnsureFolder("Assets/_TacSim", "Generated");

            AnimationClip idleClip = FindImportedIdaFaberIdleClip() ?? CreateBreathingIdleClip();
            AnimationClip walkClip = CreateStrideClip("TacSim_Walk", strideAngle: 26f, kneeBend: -32f, armSwing: 20f, periodSec: 0.92f);
            AnimationClip runClip = CreateStrideClip("TacSim_Run", strideAngle: 44f, kneeBend: -55f, armSwing: 36f, periodSec: 0.64f);
            AnimationClip seatedClip = CreateSeatedHelmClip();

            var controller = AnimatorController.CreateAnimatorControllerAtPath(ControllerPath);
            controller.AddParameter("Speed", AnimatorControllerParameterType.Float);
            controller.AddParameter("IsSprinting", AnimatorControllerParameterType.Bool);
            controller.AddParameter("Seated", AnimatorControllerParameterType.Bool);

            var sm = controller.layers[0].stateMachine;
            var stIdle = sm.AddState("Idle", new Vector3(260f, 110f, 0f));
            stIdle.motion = idleClip;

            var stWalk = sm.AddState("Walk", new Vector3(520f, 50f, 0f));
            stWalk.motion = walkClip;

            var stRun = sm.AddState("Run", new Vector3(520f, 180f, 0f));
            stRun.motion = runClip;

            var stSeated = sm.AddState("SeatedHelm", new Vector3(260f, 240f, 0f));
            stSeated.motion = seatedClip;

            sm.defaultState = stIdle;

            // Idle -> Walk
            var tIdleWalk = stIdle.AddTransition(stWalk);
            tIdleWalk.hasExitTime = false;
            tIdleWalk.duration = 0.16f;
            tIdleWalk.AddCondition(AnimatorConditionMode.Greater, 0.2f, "Speed");

            // Walk -> Idle
            var tWalkIdle = stWalk.AddTransition(stIdle);
            tWalkIdle.hasExitTime = false;
            tWalkIdle.duration = 0.18f;
            tWalkIdle.AddCondition(AnimatorConditionMode.Less, 0.15f, "Speed");

            // Walk -> Run
            var tWalkRun = stWalk.AddTransition(stRun);
            tWalkRun.hasExitTime = false;
            tWalkRun.duration = 0.15f;
            tWalkRun.AddCondition(AnimatorConditionMode.If, 0f, "IsSprinting");
            tWalkRun.AddCondition(AnimatorConditionMode.Greater, 5.2f, "Speed");

            // Run -> Walk
            var tRunWalk = stRun.AddTransition(stWalk);
            tRunWalk.hasExitTime = false;
            tRunWalk.duration = 0.18f;
            tRunWalk.AddCondition(AnimatorConditionMode.IfNot, 0f, "IsSprinting");

            var tRunIdle = stRun.AddTransition(stIdle);
            tRunIdle.hasExitTime = false;
            tRunIdle.duration = 0.2f;
            tRunIdle.AddCondition(AnimatorConditionMode.Less, 0.2f, "Speed");

            // Any -> SeatedHelm
            var tAnySeat = sm.AddAnyStateTransition(stSeated);
            tAnySeat.hasExitTime = false;
            tAnySeat.canTransitionToSelf = false;
            tAnySeat.duration = 0.22f;
            tAnySeat.AddCondition(AnimatorConditionMode.If, 0f, "Seated");

            var tSeatIdle = stSeated.AddTransition(stIdle);
            tSeatIdle.hasExitTime = false;
            tSeatIdle.duration = 0.22f;
            tSeatIdle.AddCondition(AnimatorConditionMode.IfNot, 0f, "Seated");

            EditorUtility.SetDirty(controller);
            AssetDatabase.SaveAssets();
            return controller;
        }

        private static AnimationClip FindImportedIdaFaberIdleClip()
        {
            string[] candidates =
            {
                "Assets/IdaFaber/Demo/A_Idle_F.fbx",
                "Assets/IdaFaber/Demo/A_Idle_M.fbx"
            };
            foreach (string path in candidates)
            {
                if (!File.Exists(path)) continue;
                Object[] subAssets = AssetDatabase.LoadAllAssetsAtPath(path);
                foreach (var obj in subAssets)
                {
                    if (obj is AnimationClip clip && !clip.name.StartsWith("__preview__"))
                        return clip;
                }
            }
            return null;
        }

        private static AnimationClip CreateBreathingIdleClip()
        {
            string path = GeneratedFolder + "/TacSim_Idle.anim";
            var clip = new AnimationClip { name = "TacSim_Idle" };
            SetLoopTime(clip, true);
            SetHumanoidRootPose(clip, pelvisHeightM: 0.94f, duration: 2.8f);

            SetMuscleSinusoid(clip, "Spine Front-Back", 0.03f, 2.8f, phaseOffset: 0f);
            SetMuscleSinusoid(clip, "Chest Front-Back", 0.04f, 2.8f, phaseOffset: 0.4f);
            SetMuscleConstant(clip, "Left Arm Down-Up", -0.35f, 2.8f);
            SetMuscleConstant(clip, "Right Arm Down-Up", -0.35f, 2.8f);

            SaveOrReplaceClip(clip, path);
            return AssetDatabase.LoadAssetAtPath<AnimationClip>(path);
        }

        private static AnimationClip CreateStrideClip(string clipName, float strideAngle, float kneeBend, float armSwing, float periodSec)
        {
            string path = $"{GeneratedFolder}/{clipName}.anim";
            var clip = new AnimationClip { name = clipName };
            SetLoopTime(clip, true);
            SetHumanoidRootPose(clip, pelvisHeightM: 0.94f, duration: periodSec);

            float normLeg = Mathf.Clamp(strideAngle / 90f, -1f, 1f);
            float normKnee = Mathf.Clamp(kneeBend / 90f, -1f, 1f);
            float normArm = Mathf.Clamp(armSwing / 90f, -1f, 1f);

            SetMuscleSinusoid(clip, "Left Upper Leg Front-Back", normLeg, periodSec, 0f);
            SetMuscleSinusoid(clip, "Right Upper Leg Front-Back", normLeg, periodSec, Mathf.PI);
            SetMuscleSinusoid(clip, "Left Lower Leg Stretch", normKnee * 0.5f, periodSec, Mathf.PI * 0.5f, dcOffset: -0.2f);
            SetMuscleSinusoid(clip, "Right Lower Leg Stretch", normKnee * 0.5f, periodSec, Mathf.PI * 1.5f, dcOffset: -0.2f);

            SetMuscleConstant(clip, "Left Arm Down-Up", -0.32f, periodSec);
            SetMuscleConstant(clip, "Right Arm Down-Up", -0.32f, periodSec);
            SetMuscleSinusoid(clip, "Left Arm Front-Back", normArm, periodSec, Mathf.PI);
            SetMuscleSinusoid(clip, "Right Arm Front-Back", normArm, periodSec, 0f);

            SaveOrReplaceClip(clip, path);
            return AssetDatabase.LoadAssetAtPath<AnimationClip>(path);
        }

        private static AnimationClip CreateSeatedHelmClip()
        {
            string path = GeneratedFolder + "/TacSim_SeatedHelm.anim";
            var clip = new AnimationClip { name = "TacSim_SeatedHelm" };
            SetLoopTime(clip, true);
            SetHumanoidRootPose(clip, pelvisHeightM: 0.60f, duration: 1.0f);

            SetMuscleConstant(clip, "Left Upper Leg Front-Back", 0.78f, 1.0f);
            SetMuscleConstant(clip, "Right Upper Leg Front-Back", 0.78f, 1.0f);
            SetMuscleConstant(clip, "Left Lower Leg Stretch", -0.75f, 1.0f);
            SetMuscleConstant(clip, "Right Lower Leg Stretch", -0.75f, 1.0f);
            SetMuscleConstant(clip, "Left Arm Front-Back", 0.45f, 1.0f);
            SetMuscleConstant(clip, "Right Arm Front-Back", 0.45f, 1.0f);
            SetMuscleConstant(clip, "Left Forearm Stretch", 0.35f, 1.0f);
            SetMuscleConstant(clip, "Right Forearm Stretch", 0.35f, 1.0f);

            SaveOrReplaceClip(clip, path);
            return AssetDatabase.LoadAssetAtPath<AnimationClip>(path);
        }

        private static void SetHumanoidRootPose(AnimationClip clip, float pelvisHeightM, float duration)
        {
            SetMuscleConstant(clip, "RootT.x", 0f, duration);
            SetMuscleConstant(clip, "RootT.y", pelvisHeightM, duration);
            SetMuscleConstant(clip, "RootT.z", 0f, duration);
            SetMuscleConstant(clip, "RootQ.x", 0f, duration);
            SetMuscleConstant(clip, "RootQ.y", 0f, duration);
            SetMuscleConstant(clip, "RootQ.z", 0f, duration);
            SetMuscleConstant(clip, "RootQ.w", 1f, duration);
        }

        private static void SetMuscleSinusoid(AnimationClip clip, string muscleName, float amplitude, float duration, float phaseOffset, float dcOffset = 0f)
        {
            var curve = new AnimationCurve();
            int samples = 9;
            for (int i = 0; i <= samples; i++)
            {
                float t = (i / (float)samples) * duration;
                float rad = (i / (float)samples) * Mathf.PI * 2f + phaseOffset;
                curve.AddKey(new Keyframe(t, dcOffset + Mathf.Sin(rad) * amplitude));
            }
            clip.SetCurve(string.Empty, typeof(Animator), muscleName, curve);
        }

        private static void SetMuscleConstant(AnimationClip clip, string muscleName, float value, float duration)
        {
            var curve = AnimationCurve.Constant(0f, duration, value);
            clip.SetCurve(string.Empty, typeof(Animator), muscleName, curve);
        }

        private static void SetLoopTime(AnimationClip clip, bool loop)
        {
            var so = new SerializedObject(clip);
            var settings = so.FindProperty("m_AnimationClipSettings");
            if (settings != null)
            {
                var loopProp = settings.FindPropertyRelative("m_LoopTime");
                if (loopProp != null) loopProp.boolValue = loop;
                var loopPose = settings.FindPropertyRelative("m_LoopBlend");
                if (loopPose != null) loopPose.boolValue = loop;
            }
            so.ApplyModifiedPropertiesWithoutUndo();
        }

        private static void SaveOrReplaceClip(AnimationClip clip, string assetPath)
        {
            var existing = AssetDatabase.LoadAssetAtPath<AnimationClip>(assetPath);
            if (existing != null)
            {
                EditorUtility.CopySerialized(clip, existing);
                EditorUtility.SetDirty(existing);
            }
            else
            {
                AssetDatabase.CreateAsset(clip, assetPath);
            }
        }

        private static void EnsureFolder(string parent, string child)
        {
            string full = $"{parent}/{child}";
            if (!AssetDatabase.IsValidFolder(full))
                AssetDatabase.CreateFolder(parent, child);
        }
    }
}
