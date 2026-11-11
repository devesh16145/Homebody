// Homebody for JetBrains editors (IntelliJ IDEA, PyCharm, WebStorm and the rest).
// Plain Java, no extra libraries: what it needs (JSON, the editor's own interface) is part of
// the editor it is installed into.
plugins {
    id("java")
    id("org.jetbrains.intellij.platform") version "2.2.1"
}

group = "app.homebody"
version = "0.1.0"

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        intellijIdeaCommunity("2024.2.4")
    }
}

java {
    sourceCompatibility = JavaVersion.VERSION_21
    targetCompatibility = JavaVersion.VERSION_21
}

intellijPlatform {
    buildSearchableOptions = false
    pluginConfiguration {
        ideaVersion {
            sinceBuild = "242"
            untilBuild = provider { null }
        }
    }
}
