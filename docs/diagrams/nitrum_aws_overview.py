from pathlib import Path

from diagrams import Cluster, Diagram, Edge
from diagrams.aws.compute import EC2, EC2AutoScaling
from diagrams.aws.database import DynamodbTable
from diagrams.aws.management import CloudwatchLogs, ParameterStore
from diagrams.aws.network import IGW, NLB, NATGateway, PrivateSubnet, PublicSubnet, VPC
from diagrams.aws.security import IAMRole, KMS
from diagrams.aws.storage import S3
from diagrams.onprem.client import User
from diagrams.onprem.compute import Server


BASE_DIR = Path(__file__).resolve().parent
OUTPUT_DIR = BASE_DIR / "output"


def build_diagram() -> None:
    OUTPUT_DIR.mkdir(exist_ok=True)

    graph_attr = {
        "pad": "0.4",
        "nodesep": "0.7",
        "ranksep": "0.9",
        "splines": "ortho",
    }

    with Diagram(
        "Nitrum AWS Deployment Overview",
        filename=str(OUTPUT_DIR / "nitrum-aws-overview"),
        outformat="png",
        show=False,
        direction="LR",
        graph_attr=graph_attr,
    ):
        client = User("external client")
        acme_ca = Server("Let's Encrypt\nACME CA")

        with Cluster("Developer workstation"):
            cli = Server("nitrum CLI")
            build = Server("Docker build +\nnitro-cli build-enclave")
            deploy = Server("cloud deploy\nCloudFormation update")

        cli >> Edge(label="nitrum build") >> build
        cli >> Edge(label="nitrum cloud deploy") >> deploy

        with Cluster("AWS account"):
            eif_bucket = S3("EIF artifacts")
            kms = KMS("project KMS key")
            dynamodb = DynamodbTable("data-plane state")
            ssm = ParameterStore("app env params")
            logs = CloudwatchLogs("control-plane +\ndata-plane logs")

            with Cluster("Nitrum VPC"):
                vpc = VPC("network + endpoints")
                igw = IGW("internet gateway")
                nat = NATGateway("NAT gateway")

                with Cluster("Public subnets"):
                    public = PublicSubnet("entry subnet")
                    nlb = NLB("public NLB\nTCP 80 / 443")

                with Cluster("Private subnets"):
                    private = PrivateSubnet("workload subnet")
                    asg = EC2AutoScaling("Nitro EC2 ASG")

                    with Cluster("Nitro-enabled EC2 instance"):
                        host = EC2("Nitro EC2 host")
                        control_plane = Server(
                            "control-plane:\nKick off networking,\npull artifact and run/monitor"
                        )
                        role = IAMRole("instance role\nIMDSv2 credentials")

                        with Cluster("Nitro Enclave"):
                            ingress = Server("data-plane ingress\nTLS + ACME + attestation")
                            user_service = Server("user service")
                            crypto = Server("crypto API\nlocalhost:3000\nattestation + encrypt/decrypt")

        build >> Edge(label="upload EIF") >> eif_bucket
        deploy >> Edge(label="provisions") >> asg
        deploy >> kms
        deploy >> dynamodb
        deploy >> ssm
        deploy >> logs
        deploy >> vpc

        igw >> public >> nlb
        nlb >> Edge(label="TCP passthrough") >> asg >> host
        private >> asg
        nat >> private
        vpc - igw
        vpc - nat

        client >> Edge(label="HTTPS") >> nlb
        host >> control_plane
        eif_bucket >> Edge(label="artifact pull") >> control_plane
        control_plane >> Edge(label="establish vsock / TAP + supervise restart") >> ingress
        ingress >> Edge(label="reverse proxy app traffic") >> user_service
        user_service >> Edge(label="POST /attestation") >> crypto
        user_service >> Edge(label="POST /encrypt, /decrypt") >> crypto
        ingress >> Edge(label="ACME challenge + finalize") >> acme_ca
        acme_ca >> Edge(label="issued certificate chain") >> ingress

        ingress >> Edge(label="platform key") >> kms
        ingress >> Edge(label="state + cert blobs") >> dynamodb
        ingress >> Edge(label="load env") >> ssm
        ingress >> Edge(label="ship logs") >> logs
        ingress >> Edge(label="IMDS via gvproxy") >> role


if __name__ == "__main__":
    build_diagram()
